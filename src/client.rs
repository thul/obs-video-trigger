use crate::args::Options;
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    net::TcpStream,
    path::PathBuf,
    time::Duration,
};

#[derive(Debug)]
pub struct CliError {
    pub message: String,
    pub code: i32,
}

impl CliError {
    fn new(message: impl Into<String>, code: i32) -> Self {
        Self {
            message: message.into(),
            code,
        }
    }
}

fn request(options: &Options, path: &str, body: Option<Value>) -> Result<(u16, Value), CliError> {
    let address = format!("{}:{}", options.host, options.port);
    let mut stream = TcpStream::connect(&address).map_err(|error| CliError::new(
        format!("no daemon listening on http://{address} ({error})\nStart it by running obs-video-trigger with no arguments."), 1,
    ))?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let content = body.map(|value| value.to_string()).unwrap_or_default();
    let method = if content.is_empty() { "GET" } else { "POST" };
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{content}",
        options.host,
        content.len(),
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|e| CliError::new(e.to_string(), 1))?;
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .map_err(|e| CliError::new(e.to_string(), 1))?;
    let text = String::from_utf8_lossy(&response);
    let (head, body) = text
        .split_once("\r\n\r\n")
        .ok_or_else(|| CliError::new("unexpected daemon reply", 1))?;
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(500);
    let value = serde_json::from_str(body).map_err(|_| {
        CliError::new(
            format!("unexpected reply from http://{address} (HTTP {status})"),
            1,
        )
    })?;
    Ok((status, value))
}

fn absolute_existing(raw: &str, folder: bool) -> Result<PathBuf, CliError> {
    let path = fs::canonicalize(raw).map_err(|_| {
        CliError::new(
            format!(
                "no such {}: {}",
                if folder { "folder" } else { "file" },
                std::path::absolute(raw)
                    .unwrap_or_else(|_| raw.into())
                    .display()
            ),
            2,
        )
    })?;
    let metadata = fs::metadata(&path).map_err(|e| CliError::new(e.to_string(), 2))?;
    if folder && !metadata.is_dir() {
        return Err(CliError::new(
            format!("not a folder: {}", path.display()),
            2,
        ));
    }
    if !folder && !metadata.is_file() {
        return Err(CliError::new(format!("not a file: {}", path.display()), 2));
    }
    Ok(path)
}

pub fn run(options: &Options) -> Result<String, CliError> {
    use crate::args::Command;
    let (status, result) = match options.command {
        Command::Play => {
            let file = absolute_existing(&options.file, false)?;
            request(
                options,
                "/play",
                Some(json!({ "file": file, "volume": options.volume, "fit": options.fit })),
            )?
        }
        Command::PlayFolder => {
            let folder = absolute_existing(&options.folder, true)?;
            request(
                options,
                "/play-folder",
                Some(json!({ "folder": folder, "volume": options.volume, "fit": options.fit })),
            )?
        }
        Command::Stop => request(options, "/stop", None)?,
        Command::Status => request(options, "/status", None)?,
        Command::AudioDevices => request(options, "/audio-devices", None)?,
        _ => unreachable!(),
    };
    if status >= 400 || result.get("ok") != Some(&Value::Bool(true)) {
        return Err(CliError::new(
            result
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("request failed"),
            1,
        ));
    }
    match options.command {
        Command::Play => {
            if result["active"].as_bool() == Some(false) {
                return Ok(format!(
                    "stopped {}",
                    PathBuf::from(&options.file)
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                ));
            }
            let overlays = result["overlays"].as_u64().unwrap_or(0);
            if overlays == 0 {
                return Err(CliError::new(
                    "warning: no overlay connected - is the OBS browser source running?",
                    1,
                ));
            }
            Ok(format!(
                "triggered {} on {overlays} overlay(s)",
                PathBuf::from(&options.file)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
            ))
        }
        Command::PlayFolder => Ok(if result["active"].as_bool().unwrap_or(false) {
            format!(
                "started random playback of {} clip(s) from {}",
                result["clips"], options.folder
            )
        } else {
            format!("stopped random playback of {}", options.folder)
        }),
        Command::AudioDevices => {
            let mut lines = vec![format!(
                "Audio output devices seen by the overlay ({} connected):",
                result["overlays"]
            )];
            if let Some(devices) = result["devices"].as_array() {
                for device in devices {
                    lines.push(format!(
                        "  {}   [{}]",
                        device["label"].as_str().unwrap_or("(unnamed)"),
                        device["id"].as_str().unwrap_or("")
                    ));
                }
            }
            Ok(lines.join("\n"))
        }
        _ => Ok(result.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::{Command, DEFAULT_HOST};
    use std::net::TcpListener;
    use std::thread;

    fn options(command: Command, port: u16) -> Options {
        Options {
            command,
            file: String::new(),
            folder: String::new(),
            host: DEFAULT_HOST.into(),
            port,
            volume: 1.0,
            fit: "contain".into(),
            tray: false,
            audio_device: String::new(),
        }
    }

    fn fake(reply: &'static str) -> u16 {
        let listener = TcpListener::bind((DEFAULT_HOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 2048];
            let _ = stream.read(&mut request);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                reply.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
        });
        port
    }

    #[test]
    fn status_returns_daemon_json() {
        let output = run(&options(
            Command::Status,
            fake(r#"{"ok":true,"overlays":0}"#),
        ))
        .unwrap();
        assert!(output.contains("\"ok\":true"));
    }

    #[test]
    fn non_json_daemon_reply_is_clear() {
        let error = run(&options(Command::Status, fake("not json"))).unwrap_err();
        assert!(error.message.contains("unexpected reply"));
    }

    #[test]
    fn missing_play_file_fails_before_connecting() {
        let mut options = options(Command::Play, 1);
        options.file = std::env::temp_dir()
            .join("absent-clip.webm")
            .to_string_lossy()
            .into_owned();
        let error = run(&options).unwrap_err();
        assert_eq!(error.code, 2);
        assert!(error.message.contains("no such file"));
    }
}
