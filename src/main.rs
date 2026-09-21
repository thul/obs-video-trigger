#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod args;
mod client;
mod console;
mod diagnostics;
mod overlay;
mod server;
mod tray;

use args::{Command, parse_args};
use std::{
    env, process,
    sync::mpsc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio_util::sync::CancellationToken;

const HELP: &str = r#"obs-video-trigger - play a video once in an OBS browser source

Usage:
  obs-video-trigger [daemon options]         Start the daemon (default).
  obs-video-trigger --play <file> [options]  Play a video file in the overlay.
  obs-video-trigger <file> [options]         Same as --play.
  obs-video-trigger --play-folder <folder>   Toggle random folder playback.
  obs-video-trigger --stop                   Hide the overlay immediately.
  obs-video-trigger --status                 Print daemon state as JSON.
  obs-video-trigger --list-audio-devices     List overlay audio output devices.
  obs-video-trigger --help                   Show this help.

Daemon options: --host <addr>, --port <number>, --audio-device <name>, --no-tray
Play options:   --volume <0..1>, --fit <contain|cover|fill>
"#;

fn token() -> String {
    if let Ok(token) = env::var("OBS_VIDEO_TRIGGER_TOKEN") {
        return token;
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{:x}-{:x}", process::id(), now)
}

#[tokio::main]
async fn main() {
    let argv: Vec<String> = env::args().skip(1).collect();
    let options = match parse_args(&argv) {
        Ok(options) => options,
        Err(error) => {
            console::err(&format!("error: {error}"));
            process::exit(2);
        }
    };
    if options.command == Command::Help {
        console::out(HELP);
        return;
    }
    if options.command != Command::Daemon {
        match client::run(&options) {
            Ok(output) => console::out(&output),
            Err(error) => {
                console::err(&if error.message.starts_with("warning:") {
                    error.message
                } else {
                    format!("error: {}", error.message)
                });
                process::exit(error.code);
            }
        }
        return;
    }

    diagnostics::init();
    let shutdown_token = token();
    let cancel = CancellationToken::new();
    let base = display_base(&options.host, options.port);
    let (notification_tx, notification_rx) = mpsc::channel();
    let (playback_tx, playback_rx) = mpsc::channel();
    let tray = if options.tray {
        tray::start(base.clone(), cancel.clone(), notification_rx, playback_rx)
    } else {
        None
    };
    console::out("obs-video-trigger daemon running.");
    console::out(&format!(
        "  Diagnostics log   : {}",
        diagnostics::path().display()
    ));
    console::out(&format!("  OBS browser source : {base}/overlay"));
    console::out("  Trigger a clip     : obs-video-trigger --play \"C:\\path\\to\\clip.webm\"");

    let config = server::DaemonConfig {
        host: options.host,
        port: options.port,
        audio_device: options.audio_device,
        shutdown_token,
        cancel: cancel.clone(),
        notifications: tray.is_some().then_some(notification_tx),
        playback: tray.is_some().then_some(playback_tx),
    };
    tokio::select! {
        result = server::run(config) => if let Err(error) = result { console::err(&format!("error: {error}")); process::exit(1); },
        _ = tokio::signal::ctrl_c() => cancel.cancel(),
    }
    drop(tray);
}

fn display_base(host: &str, port: u16) -> String {
    match host {
        "0.0.0.0" => format!("http://127.0.0.1:{port}"),
        "::" | "[::]" => format!("http://[::1]:{port}"),
        _ if host.contains(':') && !host.starts_with('[') => format!("http://[{host}]:{port}"),
        _ => format!("http://{host}:{port}"),
    }
}
