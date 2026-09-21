use std::{
    env,
    fs::{File, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

static LOG: OnceLock<Mutex<Option<File>>> = OnceLock::new();
static PATH: OnceLock<PathBuf> = OnceLock::new();

pub fn path() -> &'static PathBuf {
    PATH.get_or_init(|| {
        let root = env::var_os("XDG_STATE_HOME")
            .or_else(|| env::var_os("LOCALAPPDATA"))
            .map(PathBuf::from)
            .unwrap_or_else(env::temp_dir);
        root.join("obs-video-trigger").join("diagnostics.log")
    })
}

pub fn init() {
    let _ = file();
    log(&format!(
        "daemon-start version={} pid={}",
        env!("CARGO_PKG_VERSION"),
        std::process::id()
    ));
}

fn file() -> &'static Mutex<Option<File>> {
    LOG.get_or_init(|| {
        let path = path();
        let opened = path
            .parent()
            .and_then(|parent| std::fs::create_dir_all(parent).ok())
            .and_then(|_| OpenOptions::new().create(true).append(true).open(path).ok());
        Mutex::new(opened)
    })
}

pub fn log(message: &str) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    if let Some(output) = file().lock().unwrap().as_mut() {
        let _ = writeln!(
            output,
            "{}.{:03} {message}",
            now.as_secs(),
            now.subsec_millis()
        );
        let _ = output.flush();
    }
}
