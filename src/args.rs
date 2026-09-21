use std::fmt;

pub const DEFAULT_HOST: &str = "127.0.0.1";
pub const DEFAULT_PORT: u16 = 4466;

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Daemon,
    Play,
    PlayFolder,
    Stop,
    Status,
    AudioDevices,
    Help,
}

#[derive(Clone, Debug)]
pub struct Options {
    pub command: Command,
    pub file: String,
    pub folder: String,
    pub host: String,
    pub port: u16,
    pub volume: f64,
    pub fit: String,
    pub tray: bool,
    pub audio_device: String,
}

#[derive(Debug)]
pub struct UsageError(pub String);

impl fmt::Display for UsageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn value(args: &[String], index: &mut usize, option: &str) -> Result<String, UsageError> {
    *index += 1;
    args.get(*index)
        .cloned()
        .ok_or_else(|| UsageError(format!("{option} needs a value")))
}

pub fn parse_args(args: &[String]) -> Result<Options, UsageError> {
    let mut out = Options {
        command: Command::Daemon,
        file: String::new(),
        folder: String::new(),
        host: DEFAULT_HOST.into(),
        port: DEFAULT_PORT,
        volume: 1.0,
        fit: "contain".into(),
        tray: true,
        audio_device: String::new(),
    };
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "--play" | "-p" => {
                out.command = Command::Play;
                out.file = value(args, &mut i, arg)?;
            }
            "--play-folder" => {
                out.command = Command::PlayFolder;
                out.folder = value(args, &mut i, arg)?;
            }
            "--stop" => out.command = Command::Stop,
            "--status" => out.command = Command::Status,
            "--list-audio-devices" => out.command = Command::AudioDevices,
            "--daemon" | "--serve" => out.command = Command::Daemon,
            "--host" => out.host = value(args, &mut i, arg)?,
            "--port" => {
                let raw = value(args, &mut i, arg)?;
                out.port = raw.parse().map_err(|_| {
                    UsageError("--port needs a whole number between 1 and 65535".into())
                })?;
                if out.port == 0 {
                    return Err(UsageError(
                        "--port needs a whole number between 1 and 65535".into(),
                    ));
                }
            }
            "--volume" => {
                let raw = value(args, &mut i, arg)?;
                out.volume = raw
                    .parse()
                    .map_err(|_| UsageError("--volume needs a number between 0 and 1".into()))?;
                if !(0.0..=1.0).contains(&out.volume) {
                    return Err(UsageError("--volume needs a number between 0 and 1".into()));
                }
            }
            "--fit" => {
                out.fit = value(args, &mut i, arg)?;
                if !matches!(out.fit.as_str(), "contain" | "cover" | "fill") {
                    return Err(UsageError("--fit must be contain, cover or fill".into()));
                }
            }
            "--audio-device" => out.audio_device = value(args, &mut i, arg)?,
            "--interrupt" => {}
            "--no-tray" => out.tray = false,
            "--help" | "-h" => out.command = Command::Help,
            _ if arg.starts_with('-') => return Err(UsageError(format!("unknown option: {arg}"))),
            _ => {
                out.command = Command::Play;
                out.file = arg.clone();
            }
        }
        i += 1;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).into()).collect()
    }

    #[test]
    fn defaults_to_daemon() {
        assert_eq!(parse_args(&[]).unwrap().command, Command::Daemon);
    }
    #[test]
    fn parses_play_folder() {
        let out = parse_args(&strings(&["--play-folder", r"C:\clips", "--fit", "cover"])).unwrap();
        assert_eq!(out.command, Command::PlayFolder);
        assert_eq!(out.folder, r"C:\clips");
        assert_eq!(out.fit, "cover");
    }
    #[test]
    fn rejects_bad_volume() {
        assert!(parse_args(&strings(&["--volume", "2"])).is_err());
    }
    #[test]
    fn rejects_bad_ports() {
        assert!(parse_args(&strings(&["--port", "0"])).is_err());
        assert!(parse_args(&strings(&["--port", "65536"])).is_err());
    }
}
