use std::{
    collections::HashMap,
    convert::Infallible,
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    Router,
    body::Body,
    extract::{
        Path as AxumPath, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, Method, Request, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response, Sse, sse::Event},
    routing::{get, post},
};
use futures_util::{SinkExt, StreamExt, stream};
use http_body_util::BodyExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncSeekExt},
    net::TcpListener,
    sync::broadcast,
};
use tokio_stream::wrappers::BroadcastStream;
use tokio_util::{io::ReaderStream, sync::CancellationToken};

use crate::overlay;

const FORMATS: &[&str] = &["mp4", "webm", "mov", "m4v", "mkv", "ogv"];

#[derive(Clone)]
pub struct DaemonConfig {
    pub host: String,
    pub port: u16,
    pub audio_device: String,
    pub shutdown_token: String,
    pub cancel: CancellationToken,
    pub notifications: Option<mpsc::Sender<(String, bool)>>,
    pub playback: Option<mpsc::Sender<crate::tray::PlaybackUpdate>>,
}

#[derive(Clone, Serialize)]
struct AudioDevice {
    id: String,
    label: String,
}

#[derive(Clone)]
struct FolderPlayback {
    folder: PathBuf,
    files: Vec<PathBuf>,
    remaining: Vec<PathBuf>,
    queued: Option<(PathBuf, usize)>,
    current_index: usize,
    volume: f64,
    fit: String,
    token: u64,
}

struct Inner {
    next_id: u64,
    next_token: u64,
    allowed: HashMap<String, PathBuf>,
    ids: HashMap<PathBuf, String>,
    folder: Option<FolderPlayback>,
    audio_devices: Vec<AudioDevice>,
    active_event: Option<String>,
}

#[derive(Clone)]
pub struct AppState {
    inner: Arc<Mutex<Inner>>,
    events: broadcast::Sender<String>,
    config: DaemonConfig,
    overlay_html: Arc<String>,
    overlay_version: Arc<String>,
}

#[derive(Default, Deserialize)]
struct Params {
    file: Option<String>,
    clip: Option<String>,
    folder: Option<String>,
    volume: Option<f64>,
    fit: Option<String>,
    token: Option<u64>,
    devices: Option<Vec<AudioDeviceInput>>,
    error: Option<String>,
    time: Option<f64>,
    duration: Option<f64>,
    paused: Option<bool>,
}

#[derive(Deserialize)]
struct AudioDeviceInput {
    id: String,
    #[serde(default)]
    label: String,
}

fn reply(value: Value, status: StatusCode) -> Response {
    (
        status,
        [(header::CACHE_CONTROL, "no-store")],
        axum::Json(value),
    )
        .into_response()
}

fn ok(value: Value) -> Response {
    reply(value, StatusCode::OK)
}

fn fail(state: &AppState, message: String, status: StatusCode) -> Response {
    crate::console::err(&format!("error: {message}"));
    if let Some(notifications) = &state.config.notifications {
        let _ = notifications.send((message.clone(), true));
    }
    reply(json!({ "ok": false, "error": message }), status)
}

async fn params(request: Request<Body>) -> Params {
    if request.method() == Method::POST {
        let bytes = request
            .into_body()
            .collect()
            .await
            .map(|b| b.to_bytes())
            .unwrap_or_default();
        serde_json::from_slice(&bytes).unwrap_or_default()
    } else {
        let mut params = Params::default();
        for (key, value) in
            url::form_urlencoded::parse(request.uri().query().unwrap_or_default().as_bytes())
        {
            match key.as_ref() {
                "file" => params.file = Some(value.into_owned()),
                "clip" => params.clip = Some(value.into_owned()),
                "folder" => params.folder = Some(value.into_owned()),
                "volume" => params.volume = value.parse().ok(),
                "fit" => params.fit = Some(value.into_owned()),
                "token" => params.token = value.parse().ok(),
                "error" => params.error = Some(value.into_owned()),
                "time" => params.time = value.parse().ok(),
                "duration" => params.duration = value.parse().ok(),
                "paused" => params.paused = value.parse().ok(),
                _ => {}
            }
        }
        params
    }
}

fn supported(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| FORMATS.contains(&e.to_ascii_lowercase().as_str()))
}

fn shuffle(files: &mut [PathBuf]) {
    let mut state = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    for index in (1..files.len()).rev() {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        files.swap(index, state as usize % (index + 1));
    }
}

fn play_event(state: &AppState, path: &Path, volume: f64, fit: &str) -> (Value, usize) {
    let mut inner = state.inner.lock().unwrap();
    let id = if let Some(id) = inner.ids.get(path) {
        id.clone()
    } else {
        inner.next_id += 1;
        let id = inner.next_id.to_string();
        inner.ids.insert(path.to_path_buf(), id.clone());
        inner.allowed.insert(id.clone(), path.to_path_buf());
        id
    };
    inner.next_token += 1;
    let token = inner.next_token;
    let event = json!({
        "type": "play", "id": id, "name": path.file_name().unwrap_or_default().to_string_lossy(),
        "file": path.to_string_lossy(), "src": format!("/media/{id}"),
        "volume": volume.clamp(0.0, 1.0), "fit": fit, "sink": state.config.audio_device, "token": token,
    });
    inner.active_event = Some(event.to_string());
    drop(inner);
    let overlays = state.events.receiver_count();
    crate::diagnostics::log(&format!(
        "daemon play token={token} overlays={overlays} file={}",
        path.display()
    ));
    let _ = state.events.send(event.to_string());
    (event, overlays)
}

fn preload_event(state: &AppState, path: &Path, volume: f64, fit: &str) {
    let mut inner = state.inner.lock().unwrap();
    let id = if let Some(id) = inner.ids.get(path) {
        id.clone()
    } else {
        inner.next_id += 1;
        let id = inner.next_id.to_string();
        inner.ids.insert(path.to_path_buf(), id.clone());
        inner.allowed.insert(id.clone(), path.to_path_buf());
        id
    };
    let event = json!({
        "type": "preload", "id": id, "name": path.file_name().unwrap_or_default().to_string_lossy(),
        "file": path.to_string_lossy(), "src": format!("/media/{id}"),
        "volume": volume.clamp(0.0, 1.0), "fit": fit, "sink": state.config.audio_device,
    });
    drop(inner);
    crate::diagnostics::log(&format!("daemon preload file={}", path.display()));
    let _ = state.events.send(event.to_string());
}

fn active_token(state: &AppState) -> Option<u64> {
    state
        .inner
        .lock()
        .unwrap()
        .active_event
        .as_deref()
        .and_then(|event| serde_json::from_str::<Value>(event).ok())
        .and_then(|event| event["token"].as_u64())
}

fn report_playback(state: &AppState, value: &Value) {
    let token = value["token"].as_u64();
    if token.is_none() || token != active_token(state) {
        return;
    }
    if let Some(playback) = &state.config.playback {
        let (folder, index, total) = state
            .inner
            .lock()
            .unwrap()
            .folder
            .as_ref()
            .map_or((false, 0, 0), |folder| {
                (true, folder.current_index, folder.files.len())
            });
        let file = state
            .inner
            .lock()
            .unwrap()
            .active_event
            .as_deref()
            .and_then(|event| serde_json::from_str::<Value>(event).ok())
            .and_then(|event| event["file"].as_str().map(str::to_owned))
            .unwrap_or_default();
        let _ = playback.send((
            value["time"].as_f64().unwrap_or(0.0),
            value["duration"].as_f64().unwrap_or(0.0),
            !value["paused"].as_bool().unwrap_or(false),
            folder,
            index,
            total,
            file,
        ));
    }
}

fn next_folder(state: &AppState) -> Option<usize> {
    let (file, volume, fit) = {
        let mut inner = state.inner.lock().unwrap();
        let folder = inner.folder.as_mut()?;
        let (file, index) = if let Some(queued) = folder.queued.take() {
            queued
        } else {
            if folder.remaining.is_empty() {
                folder.remaining = folder.files.clone();
                shuffle(&mut folder.remaining);
            }
            let file = folder.remaining.pop()?;
            let index = folder.files.len() - folder.remaining.len();
            (file, index)
        };
        folder.current_index = index;
        crate::diagnostics::log(&format!(
            "queue select index={}/{} remaining={} file={}",
            index,
            folder.files.len(),
            folder.remaining.len(),
            file.display()
        ));
        (file, folder.volume, folder.fit.clone())
    };
    let (event, overlays) = play_event(state, &file, volume, &fit);
    if let Some(folder) = state.inner.lock().unwrap().folder.as_mut() {
        folder.token = event["token"].as_u64().unwrap_or(0);
    }
    let next = {
        let mut inner = state.inner.lock().unwrap();
        let folder = inner.folder.as_mut()?;
        if folder.remaining.is_empty() {
            folder.remaining = folder.files.clone();
            shuffle(&mut folder.remaining);
            if folder.remaining.len() > 1 && folder.remaining.last() == Some(&file) {
                let last = folder.remaining.len() - 1;
                folder.remaining.swap(0, last);
                crate::diagnostics::log("queue avoided immediate repeat at shuffle boundary");
            }
        }
        folder.remaining.pop().map(|next| {
            let index = folder.files.len() - folder.remaining.len();
            folder.queued = Some((next.clone(), index));
            (next, folder.volume, folder.fit.clone())
        })
    };
    if let Some(next) = next {
        preload_event(state, &next.0, next.1, &next.2);
    }
    Some(overlays)
}

fn finish_folder(state: &AppState) {
    crate::diagnostics::log("queue finish");
    let mut inner = state.inner.lock().unwrap();
    inner.folder = None;
    inner.active_event = None;
    drop(inner);
    let _ = state.events.send(json!({ "type": "stop" }).to_string());
    if let Some(playback) = &state.config.playback {
        let _ = playback.send((0.0, 0.0, false, false, 0, 0, String::new()));
    }
}

async fn play(State(state): State<AppState>, request: Request<Body>) -> Response {
    let params = params(request).await;
    let raw = params.file.or(params.clip).unwrap_or_default();
    if raw.is_empty() {
        return fail(&state, "missing file".into(), StatusCode::BAD_REQUEST);
    }
    let path = PathBuf::from(&raw);
    if !path.is_absolute() {
        return fail(
            &state,
            format!("absolute path required: {raw}"),
            StatusCode::BAD_REQUEST,
        );
    }
    if !path.is_file() {
        return fail(
            &state,
            format!("no such file: {}", path.display()),
            StatusCode::NOT_FOUND,
        );
    }
    {
        let mut inner = state.inner.lock().unwrap();
        let same_active_file = inner.folder.is_none()
            && inner
                .active_event
                .as_deref()
                .and_then(|event| serde_json::from_str::<Value>(event).ok())
                .and_then(|event| event["file"].as_str().map(PathBuf::from))
                .is_some_and(|active| active == path);
        if same_active_file {
            crate::diagnostics::log(&format!("daemon toggle-off file={}", path.display()));
            inner.active_event = None;
            drop(inner);
            let overlays = state.events.receiver_count();
            let _ = state.events.send(json!({ "type": "stop" }).to_string());
            if let Some(playback) = &state.config.playback {
                let _ = playback.send((0.0, 0.0, false, false, 0, 0, String::new()));
            }
            return ok(json!({
                "ok": true, "active": false, "file": path, "overlays": overlays
            }));
        }
        inner.folder = None;
    }
    if let Some(playback) = &state.config.playback {
        let _ = playback.send((
            0.0,
            0.0,
            false,
            false,
            0,
            0,
            path.to_string_lossy().into_owned(),
        ));
    }
    let (_, overlays) = play_event(
        &state,
        &path,
        params.volume.unwrap_or(1.0),
        valid_fit(params.fit.as_deref()),
    );
    ok(json!({ "ok": true, "active": true, "file": path, "overlays": overlays }))
}

fn valid_fit(fit: Option<&str>) -> &str {
    match fit {
        Some("cover") => "cover",
        Some("fill") => "fill",
        _ => "contain",
    }
}

async fn play_folder(State(state): State<AppState>, request: Request<Body>) -> Response {
    let params = params(request).await;
    let raw = params.folder.unwrap_or_default();
    if raw.is_empty() {
        return fail(&state, "missing folder".into(), StatusCode::BAD_REQUEST);
    }
    let folder = PathBuf::from(&raw);
    if !folder.is_absolute() {
        return fail(
            &state,
            format!("absolute path required: {raw}"),
            StatusCode::BAD_REQUEST,
        );
    }
    {
        let mut inner = state.inner.lock().unwrap();
        if inner.folder.as_ref().is_some_and(|p| p.folder == folder) {
            inner.folder = None;
            inner.active_event = None;
            drop(inner);
            let overlays = state.events.receiver_count();
            let _ = state.events.send(json!({ "type": "stop" }).to_string());
            return ok(
                json!({ "ok": true, "active": false, "folder": folder, "overlays": overlays }),
            );
        }
    }
    let files: Vec<PathBuf> = match std::fs::read_dir(&folder) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.is_file() && supported(p))
            .collect(),
        Err(_) => {
            return fail(
                &state,
                format!("no such folder: {}", folder.display()),
                StatusCode::NOT_FOUND,
            );
        }
    };
    if files.is_empty() {
        return fail(
            &state,
            format!("no supported video files in folder: {}", folder.display()),
            StatusCode::NOT_FOUND,
        );
    }
    let count = files.len();
    let mut remaining = files.clone();
    shuffle(&mut remaining);
    state.inner.lock().unwrap().folder = Some(FolderPlayback {
        folder: folder.clone(),
        files,
        remaining,
        queued: None,
        current_index: 0,
        volume: params.volume.unwrap_or(1.0).clamp(0.0, 1.0),
        fit: valid_fit(params.fit.as_deref()).into(),
        token: 0,
    });
    let overlays = next_folder(&state).unwrap_or(0);
    ok(
        json!({ "ok": true, "active": true, "folder": folder, "clips": count, "overlays": overlays }),
    )
}

async fn stop(State(state): State<AppState>) -> Response {
    crate::diagnostics::log("daemon stop-request");
    {
        let mut inner = state.inner.lock().unwrap();
        inner.folder = None;
        inner.active_event = None;
    }
    let overlays = state.events.receiver_count();
    let _ = state.events.send(json!({ "type": "stop" }).to_string());
    if let Some(playback) = &state.config.playback {
        let _ = playback.send((0.0, 0.0, false, false, 0, 0, String::new()));
    }
    ok(json!({ "ok": true, "overlays": overlays }))
}

async fn playback_ended(State(state): State<AppState>, request: Request<Body>) -> Response {
    let params = params(request).await;
    let (folder_active, folder_token) = state
        .inner
        .lock()
        .unwrap()
        .folder
        .as_ref()
        .map_or((false, None), |folder| (true, Some(folder.token)));
    if folder_active {
        if folder_token == params.token && next_folder(&state).is_none() {
            finish_folder(&state);
        }
    } else if params.token.is_some() && params.token == active_token(&state) {
        state.inner.lock().unwrap().active_event = None;
        if let Some(playback) = &state.config.playback {
            let _ = playback.send((0.0, 0.0, false, false, 0, 0, String::new()));
        }
    }
    ok(json!({ "ok": true, "active": state.inner.lock().unwrap().folder.is_some() }))
}

async fn playback_error(State(state): State<AppState>, request: Request<Body>) -> Response {
    let params = params(request).await;
    let message = format!(
        "{}{}",
        params
            .error
            .unwrap_or_else(|| "clip playback failed".into()),
        params.file.map(|f| format!(": {f}")).unwrap_or_default()
    );
    if let Some(notifications) = &state.config.notifications {
        let _ = notifications.send((message.clone(), true));
    }
    crate::console::err(&format!("error: {message}"));
    if state.inner.lock().unwrap().folder.is_some() {
        if next_folder(&state).is_none() {
            finish_folder(&state);
        }
    } else {
        state.inner.lock().unwrap().active_event = None;
        if let Some(playback) = &state.config.playback {
            let _ = playback.send((0.0, 0.0, false, false, 0, 0, String::new()));
        }
    }
    ok(json!({ "ok": true }))
}

async fn playback_progress(State(state): State<AppState>, request: Request<Body>) -> Response {
    let params = params(request).await;
    if let Some(playback) = &state.config.playback {
        let (folder, index, total) = state
            .inner
            .lock()
            .unwrap()
            .folder
            .as_ref()
            .map_or((false, 0, 0), |folder| {
                (true, folder.current_index, folder.files.len())
            });
        let _ = playback.send((
            params.time.unwrap_or(0.0).max(0.0),
            params.duration.unwrap_or(0.0).max(0.0),
            !params.paused.unwrap_or(false),
            folder,
            index,
            total,
            state
                .inner
                .lock()
                .unwrap()
                .active_event
                .as_deref()
                .and_then(|event| serde_json::from_str::<Value>(event).ok())
                .and_then(|event| event["file"].as_str().map(str::to_owned))
                .unwrap_or_default(),
        ));
    }
    ok(json!({ "ok": true }))
}

async fn seek(State(state): State<AppState>, request: Request<Body>) -> Response {
    let time = params(request).await.time.unwrap_or(0.0).max(0.0);
    let token = active_token(&state);
    crate::diagnostics::log(&format!("daemon seek token={token:?} time={time:.3}"));
    let _ = state
        .events
        .send(json!({ "type": "seek", "time": time, "token": token }).to_string());
    ok(json!({ "ok": true, "time": time }))
}

async fn pause(State(state): State<AppState>, request: Request<Body>) -> Response {
    let paused = params(request).await.paused.unwrap_or(true);
    let _ = state
        .events
        .send(json!({ "type": "pause", "paused": paused }).to_string());
    ok(json!({ "ok": true, "paused": paused }))
}

async fn next_video(State(state): State<AppState>) -> Response {
    if state.inner.lock().unwrap().folder.is_none() {
        return fail(
            &state,
            "folder playback is not active".into(),
            StatusCode::CONFLICT,
        );
    }
    if let Some(overlays) = next_folder(&state) {
        ok(json!({ "ok": true, "overlays": overlays }))
    } else {
        finish_folder(&state);
        ok(json!({ "ok": true, "active": false }))
    }
}

async fn status(State(state): State<AppState>) -> Response {
    let inner = state.inner.lock().unwrap();
    ok(
        json!({ "ok": true, "overlays": state.events.receiver_count(), "clips": inner.next_id,
        "port": state.config.port, "audioDevice": state.config.audio_device,
        "folderPlayback": inner.folder.as_ref().map(|f| f.folder.to_string_lossy()) }),
    )
}

async fn report_devices(State(state): State<AppState>, request: Request<Body>) -> Response {
    let params = params(request).await;
    let Some(devices) = params.devices else {
        return reply(
            json!({ "ok": false, "error": "expected {devices: [...]}" }),
            StatusCode::BAD_REQUEST,
        );
    };
    state.inner.lock().unwrap().audio_devices = devices
        .into_iter()
        .map(|d| AudioDevice {
            id: d.id,
            label: d.label,
        })
        .collect();
    ok(json!({ "ok": true }))
}

async fn audio_devices(State(state): State<AppState>) -> Response {
    if state.events.receiver_count() == 0 {
        return reply(
            json!({ "ok": false, "error": "no overlay connected - is the OBS browser source running?" }),
            StatusCode::SERVICE_UNAVAILABLE,
        );
    }
    let inner = state.inner.lock().unwrap();
    ok(
        json!({ "ok": true, "overlays": state.events.receiver_count(), "audioDevice": state.config.audio_device, "devices": inner.audio_devices }),
    )
}

async fn events(State(state): State<AppState>) -> impl IntoResponse {
    let hello = json!({ "type": "hello", "version": *state.overlay_version, "sink": state.config.audio_device }).to_string();
    let active = state
        .inner
        .lock()
        .unwrap()
        .active_event
        .clone()
        .map(|event| {
            let mut value: Value = serde_json::from_str(&event).unwrap_or_default();
            value["replay"] = json!(true);
            value.to_string()
        });
    let initial = stream::iter(
        std::iter::once(hello)
            .chain(active)
            .map(|data| Ok::<Event, Infallible>(Event::default().data(data))),
    );
    let following = BroadcastStream::new(state.events.subscribe()).filter_map(|item| async move {
        item.ok()
            .map(|data| Ok::<Event, Infallible>(Event::default().data(data)))
    });
    Sse::new(initial.chain(following)).keep_alive(axum::response::sse::KeepAlive::default())
}

async fn websocket(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    ws.on_upgrade(move |socket| websocket_session(socket, state))
}

async fn websocket_session(socket: WebSocket, state: AppState) {
    let session = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    crate::diagnostics::log(&format!("ws connect session={session:x}"));
    let (mut output, mut input) = socket.split();
    let hello = json!({ "type": "hello", "version": *state.overlay_version, "sink": state.config.audio_device }).to_string();
    if output.send(Message::Text(hello.into())).await.is_err() {
        return;
    }
    let active_event = { state.inner.lock().unwrap().active_event.clone() };
    if let Some(event) = active_event {
        let mut value: Value = serde_json::from_str(&event).unwrap_or_default();
        value["replay"] = json!(true);
        if output
            .send(Message::Text(value.to_string().into()))
            .await
            .is_err()
        {
            return;
        }
    }
    let mut events = state.events.subscribe();
    loop {
        tokio::select! {
            event = events.recv() => match event {
                Ok(event) => {
                    if let Ok(value) = serde_json::from_str::<Value>(&event) {
                        crate::diagnostics::log(&format!(
                            "ws send session={session:x} type={} token={} file={}",
                            value["type"].as_str().unwrap_or("?"),
                            value["token"].as_u64().map_or_else(|| "-".into(), |v| v.to_string()),
                            value["file"].as_str().unwrap_or("-")
                        ));
                    }
                    if output.send(Message::Text(event.into())).await.is_err() { break; }
                },
                Err(_) => break,
            },
            message = input.next() => {
                let Some(Ok(Message::Text(text))) = message else { break; };
                let Ok(value) = serde_json::from_str::<Value>(&text) else { continue; };
                match value["type"].as_str().unwrap_or("") {
                    "diagnostic" => crate::diagnostics::log(&format!("overlay session={session:x} {}", value["message"].as_str().unwrap_or("?"))),
                    "playing" => {
                        crate::diagnostics::log(&format!("ws recv session={session:x} playing token={} time={} duration={}", value["token"], value["time"], value["duration"]));
                        report_playback(&state, &value)
                    },
                    "progress" => report_playback(&state, &value),
                    "ended" => {
                        let token = value["token"].as_u64();
                        crate::diagnostics::log(&format!("ws recv session={session:x} ended token={}", value["token"]));
                        let (folder_active, current_folder_token) = state.inner.lock().unwrap().folder.as_ref()
                            .map_or((false, None), |folder| (true, Some(folder.token)));
                        if folder_active {
                            if current_folder_token == token {
                                if next_folder(&state).is_none() { finish_folder(&state); }
                            } else {
                                crate::diagnostics::log(&format!("ws ignore-stale-ended session={session:x} token={} current={}", value["token"], current_folder_token.map_or_else(|| "-".into(), |v| v.to_string())));
                            }
                        } else if token.is_some() && token == active_token(&state) {
                            state.inner.lock().unwrap().active_event = None;
                            if let Some(playback) = &state.config.playback { let _ = playback.send((0.0, 0.0, false, false, 0, 0, String::new())); }
                        } else {
                            crate::diagnostics::log(&format!("ws ignore-stale-ended session={session:x} token={} current={}", value["token"], active_token(&state).map_or_else(|| "-".into(), |v| v.to_string())));
                        }
                    },
                    "error" => {
                        let token = value["token"].as_u64();
                        crate::diagnostics::log(&format!("ws recv session={session:x} error token={} file={} error={}", value["token"], value["file"], value["error"]));
                        if token.is_none() || token != active_token(&state) { continue; }
                        let message = format!("{}: {}", value["error"].as_str().unwrap_or("browser playback failed"), value["file"].as_str().unwrap_or(""));
                        if let Some(notifications) = &state.config.notifications { let _ = notifications.send((message, true)); }
                        if state.inner.lock().unwrap().folder.is_some() && next_folder(&state).is_none() {
                            finish_folder(&state);
                        }
                    },
                    "devices" => if let Some(devices) = value["devices"].as_array() {
                        state.inner.lock().unwrap().audio_devices = devices.iter().map(|d| AudioDevice { id: d["id"].as_str().unwrap_or("").into(), label: d["label"].as_str().unwrap_or("").into() }).collect();
                    },
                    _ => {}
                }
            }
        }
    }
    crate::diagnostics::log(&format!("ws disconnect session={session:x}"));
}

async fn overlay_page(State(state): State<AppState>) -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        state.overlay_html.as_str().to_owned(),
    )
        .into_response()
}

fn content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "m4v" => "video/x-m4v",
        "mkv" => "video/x-matroska",
        "ogv" => "video/ogg",
        _ => "application/octet-stream",
    }
}

async fn media(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    headers: HeaderMap,
) -> Response {
    let path = { state.inner.lock().unwrap().allowed.get(&id).cloned() };
    let Some(path) = path else {
        return (StatusCode::NOT_FOUND, "Unknown clip").into_response();
    };
    let Ok(mut file) = tokio::fs::File::open(&path).await else {
        return (StatusCode::NOT_FOUND, "Clip no longer exists").into_response();
    };
    let size = file.metadata().await.map(|m| m.len()).unwrap_or(0);
    let requested_range = headers.get(header::RANGE);
    let range = requested_range
        .and_then(|v| v.to_str().ok())
        .and_then(|r| parse_range(r, size));
    if requested_range.is_some() && range.is_none() {
        return (
            StatusCode::RANGE_NOT_SATISFIABLE,
            [(header::CONTENT_RANGE, format!("bytes */{size}"))],
        )
            .into_response();
    }
    let (start, end, status) = range
        .map(|(s, e)| (s, e, StatusCode::PARTIAL_CONTENT))
        .unwrap_or((0, size.saturating_sub(1), StatusCode::OK));
    crate::diagnostics::log(&format!(
        "media request id={id} status={} range={start}-{end}/{size} file={}",
        status.as_u16(),
        path.display()
    ));
    if file.seek(std::io::SeekFrom::Start(start)).await.is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    let length = if size == 0 { 0 } else { end - start + 1 };
    let stream = ReaderStream::new(file.take(length));
    let mut response = Response::new(Body::from_stream(stream));
    *response.status_mut() = status;
    let h = response.headers_mut();
    h.insert(header::CONTENT_TYPE, content_type(&path).parse().unwrap());
    h.insert(header::CONTENT_LENGTH, length.into());
    h.insert(header::ACCEPT_RANGES, "bytes".parse().unwrap());
    h.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    if status == StatusCode::PARTIAL_CONTENT {
        h.insert(
            header::CONTENT_RANGE,
            format!("bytes {start}-{end}/{size}").parse().unwrap(),
        );
    }
    response
}

fn parse_range(value: &str, size: u64) -> Option<(u64, u64)> {
    let span = value.strip_prefix("bytes=")?;
    if span.contains(',') || size == 0 {
        return None;
    }
    let (left, right) = span.split_once('-')?;
    if left.is_empty() {
        let suffix: u64 = right.parse().ok()?;
        if suffix == 0 {
            return None;
        }
        return Some((size.saturating_sub(suffix), size - 1));
    }
    let start: u64 = left.parse().ok()?;
    if start >= size {
        return None;
    }
    let end = if right.is_empty() {
        size - 1
    } else {
        right.parse::<u64>().ok()?.min(size - 1)
    };
    (start <= end).then_some((start, end))
}

async fn shutdown(State(state): State<AppState>, request: Request<Body>) -> Response {
    let query: HashMap<String, String> = request
        .uri()
        .query()
        .map(|q| {
            url::form_urlencoded::parse(q.as_bytes())
                .into_owned()
                .collect()
        })
        .unwrap_or_default();
    if query.get("token") != Some(&state.config.shutdown_token) {
        return reply(
            json!({ "ok": false, "error": "bad token" }),
            StatusCode::FORBIDDEN,
        );
    }
    state.config.cancel.cancel();
    ok(json!({ "ok": true }))
}

async fn security(State(state): State<AppState>, request: Request<Body>, next: Next) -> Response {
    let loopback_bind = matches!(
        state.config.host.as_str(),
        "127.0.0.1" | "localhost" | "::1" | "[::1]"
    );
    if loopback_bind {
        let host = request
            .headers()
            .get(header::HOST)
            .and_then(|h| h.to_str().ok())
            .unwrap_or("");
        let hostname = if host.starts_with('[') {
            host.split(']').next().unwrap_or("").trim_start_matches('[')
        } else {
            host.split(':').next().unwrap_or("")
        };
        if !matches!(
            hostname.to_ascii_lowercase().as_str(),
            "127.0.0.1" | "localhost" | "::1"
        ) {
            return StatusCode::FORBIDDEN.into_response();
        }
    }
    let sensitive = matches!(
        request.uri().path(),
        "/play"
            | "/play-folder"
            | "/stop"
            | "/playback-ended"
            | "/playback-error"
            | "/playback-progress"
            | "/seek"
            | "/pause"
            | "/next"
            | "/audio-devices"
    ) && request.method() == Method::POST
        || matches!(
            request.uri().path(),
            "/play" | "/play-folder" | "/stop" | "/seek" | "/pause" | "/next"
        );
    if sensitive
        && let Some(site) = request
            .headers()
            .get("sec-fetch-site")
            .and_then(|h| h.to_str().ok())
        && !matches!(site, "none" | "same-origin")
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    next.run(request).await
}

fn app(config: DaemonConfig) -> Router {
    let (raw_overlay, version) = overlay::html();
    let (events_tx, _) = broadcast::channel(32);
    let state = AppState {
        inner: Arc::new(Mutex::new(Inner {
            next_id: 0,
            next_token: 0,
            allowed: HashMap::new(),
            ids: HashMap::new(),
            folder: None,
            audio_devices: Vec::new(),
            active_event: None,
        })),
        events: events_tx,
        config: config.clone(),
        overlay_html: Arc::new(raw_overlay),
        overlay_version: Arc::new(version),
    };
    Router::new()
        .route("/", get(overlay_page))
        .route("/overlay", get(overlay_page))
        .route("/overlay.html", get(overlay_page))
        .route("/events", get(events))
        .route("/ws", get(websocket))
        .route("/play", get(play).post(play))
        .route("/play-folder", get(play_folder).post(play_folder))
        .route("/stop", get(stop).post(stop))
        .route("/playback-ended", post(playback_ended))
        .route("/playback-error", post(playback_error))
        .route("/playback-progress", post(playback_progress))
        .route("/seek", get(seek).post(seek))
        .route("/pause", get(pause).post(pause))
        .route("/next", get(next_video).post(next_video))
        .route("/status", get(status))
        .route("/audio-devices", get(audio_devices).post(report_devices))
        .route("/media/{id}", get(media))
        .route("/shutdown", post(shutdown))
        .layer(middleware::from_fn_with_state(state.clone(), security))
        .with_state(state)
}

pub async fn run(config: DaemonConfig) -> std::io::Result<()> {
    let app = app(config.clone());
    let ip: IpAddr = config
        .host
        .parse()
        .unwrap_or(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
    let listener = TcpListener::bind(SocketAddr::new(ip, config.port)).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(config.cancel.cancelled_owned())
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use std::time::Duration;
    use tower::ServiceExt;

    fn config() -> DaemonConfig {
        DaemonConfig {
            host: "127.0.0.1".into(),
            port: 4466,
            audio_device: "Speakers".into(),
            shutdown_token: "secret".into(),
            cancel: CancellationToken::new(),
            notifications: None,
            playback: None,
        }
    }
    fn post(path: &str, value: Value) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(path)
            .header(header::HOST, "127.0.0.1:4466")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(value.to_string()))
            .unwrap()
    }
    fn get(path: &str) -> Request<Body> {
        Request::builder()
            .uri(path)
            .header(header::HOST, "127.0.0.1:4466")
            .body(Body::empty())
            .unwrap()
    }
    async fn json(response: Response) -> Value {
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
    }
    fn temp_dir() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "obs-video-trigger-rust-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[tokio::test]
    async fn overlay_is_stamped_and_has_two_players() {
        let response = app(config()).oneshot(get("/overlay")).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let html = String::from_utf8(
            response
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec(),
        )
        .unwrap();
        assert!(html.contains("player-a"));
        assert!(html.contains("player-b"));
        assert!(!html.contains("__OVERLAY_VERSION__"));
    }

    #[tokio::test]
    async fn event_stream_starts_with_matching_overlay_version_and_sink() {
        let router = app(config());
        let overlay_response = router.clone().oneshot(get("/overlay")).await.unwrap();
        let html = String::from_utf8(
            overlay_response
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec(),
        )
        .unwrap();
        let response = router.oneshot(get("/events")).await.unwrap();
        let frame = tokio::time::timeout(Duration::from_secs(1), response.into_body().frame())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let text = String::from_utf8_lossy(frame.data_ref().unwrap());
        let payload = text
            .lines()
            .find_map(|line| line.strip_prefix("data: "))
            .unwrap();
        let hello: Value = serde_json::from_str(payload).unwrap();
        assert_eq!(hello["type"], "hello");
        assert_eq!(hello["sink"], "Speakers");
        assert!(html.contains(hello["version"].as_str().unwrap()));
    }

    #[tokio::test]
    async fn play_validates_and_registers_a_file() {
        let dir = temp_dir();
        let clip = dir.join("clip.webm");
        std::fs::write(&clip, [1, 2, 3, 4]).unwrap();
        let router = app(config());
        let bad = router
            .clone()
            .oneshot(post("/play", json!({ "file": "relative.webm" })))
            .await
            .unwrap();
        assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
        let played = router
            .clone()
            .oneshot(post(
                "/play",
                json!({ "file": clip, "volume": 0.5, "fit": "cover" }),
            ))
            .await
            .unwrap();
        assert_eq!(json(played).await["ok"], true);
        let encoded: String =
            url::form_urlencoded::byte_serialize(clip.to_string_lossy().as_bytes()).collect();
        let from_url = router
            .clone()
            .oneshot(get(&format!("/play?file={encoded}&volume=0.5&fit=cover")))
            .await
            .unwrap();
        assert_eq!(json(from_url).await["ok"], true);
        let status = json(router.oneshot(get("/status")).await.unwrap()).await;
        assert_eq!(status["clips"], 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn playing_the_same_file_again_toggles_off_and_updates_manager() {
        let dir = temp_dir();
        let clip = dir.join("clip.webm");
        std::fs::write(&clip, [1, 2, 3, 4]).unwrap();
        let (playback, updates) = mpsc::channel();
        let mut daemon = config();
        daemon.playback = Some(playback);
        let router = app(daemon);

        let started = json(
            router
                .clone()
                .oneshot(post("/play", json!({ "file": clip })))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(started["active"], true);
        assert_eq!(
            updates.recv_timeout(Duration::from_millis(50)).unwrap().6,
            clip.to_string_lossy()
        );

        let stopped = json(
            router
                .oneshot(post("/play", json!({ "file": clip })))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(stopped["active"], false);
        assert_eq!(
            updates.recv_timeout(Duration::from_millis(50)).unwrap(),
            (0.0, 0.0, false, false, 0, 0, String::new())
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn media_supports_ranges_and_rejects_bad_ones() {
        let dir = temp_dir();
        let clip = dir.join("clip.webm");
        std::fs::write(&clip, (0..100u8).collect::<Vec<_>>()).unwrap();
        let router = app(config());
        json(
            router
                .clone()
                .oneshot(post("/play", json!({ "file": clip })))
                .await
                .unwrap(),
        )
        .await;
        let ranged = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/media/1")
                    .header(header::HOST, "127.0.0.1")
                    .header(header::RANGE, "bytes=10-19")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ranged.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(ranged.headers()[header::CONTENT_RANGE], "bytes 10-19/100");
        assert_eq!(
            ranged.into_body().collect().await.unwrap().to_bytes().len(),
            10
        );
        let bad = router
            .oneshot(
                Request::builder()
                    .uri("/media/1")
                    .header(header::HOST, "127.0.0.1")
                    .header(header::RANGE, "bytes=500-")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(bad.status(), StatusCode::RANGE_NOT_SATISFIABLE);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn folder_mode_starts_and_toggles_off() {
        let dir = temp_dir();
        std::fs::write(dir.join("a.webm"), [1]).unwrap();
        std::fs::write(dir.join("b.mp4"), [2]).unwrap();
        let router = app(config());
        let start = json(
            router
                .clone()
                .oneshot(post("/play-folder", json!({ "folder": dir })))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(start["active"], true);
        assert_eq!(start["clips"], 2);
        let stop = json(
            router
                .oneshot(post("/play-folder", json!({ "folder": dir })))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(stop["active"], false);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn errors_are_sent_directly_to_native_popup() {
        let (notifications, received) = mpsc::channel();
        let mut daemon = config();
        daemon.notifications = Some(notifications);
        let router = app(daemon);
        let missing = std::env::temp_dir().join("definitely-absent-video.webm");
        let response = router
            .clone()
            .oneshot(post("/play", json!({ "file": missing })))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert!(
            received
                .recv_timeout(Duration::from_millis(50))
                .unwrap()
                .0
                .contains("no such file")
        );
    }

    #[tokio::test]
    async fn overlay_audio_devices_are_reported_and_listed() {
        let router = app(config());
        let events = router.clone().oneshot(get("/events")).await.unwrap();
        assert_eq!(events.status(), StatusCode::OK);
        let report = router
            .clone()
            .oneshot(post(
                "/audio-devices",
                json!({ "devices": [{ "id": "spk", "label": "Speakers" }] }),
            ))
            .await
            .unwrap();
        assert_eq!(report.status(), StatusCode::OK);
        let listed = json(router.oneshot(get("/audio-devices")).await.unwrap()).await;
        assert_eq!(listed["audioDevice"], "Speakers");
        assert_eq!(listed["devices"][0]["id"], "spk");
        drop(events);
    }

    #[tokio::test]
    async fn cross_site_and_foreign_hosts_are_rejected() {
        let foreign_site = Request::builder()
            .method("POST")
            .uri("/stop")
            .header(header::HOST, "127.0.0.1")
            .header("sec-fetch-site", "cross-site")
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            app(config()).oneshot(foreign_site).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
        let foreign_host = Request::builder()
            .uri("/status")
            .header(header::HOST, "evil.example")
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            app(config()).oneshot(foreign_host).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
    }

    #[test]
    fn byte_ranges_match_browser_semantics() {
        assert_eq!(parse_range("bytes=10-19", 100), Some((10, 19)));
        assert_eq!(parse_range("bytes=90-", 100), Some((90, 99)));
        assert_eq!(parse_range("bytes=-10", 100), Some((90, 99)));
        assert_eq!(parse_range("bytes=500-", 100), None);
        assert_eq!(parse_range("bytes=1-2,4-5", 100), None);
    }
}
