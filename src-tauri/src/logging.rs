use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::pure::{log_line, parse_log_level};

const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

struct Logger {
    dir: PathBuf,
    level: u8,
}

static LOGGER: Mutex<Option<Logger>> = Mutex::new(None);

/// 根据配置开启/关闭日志。关闭时所有写入都是空操作。
pub fn configure(dir: PathBuf, enabled: bool, level: &str) {
    let mut guard = LOGGER.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    *guard = if enabled {
        Some(Logger {
            dir,
            level: parse_log_level(level),
        })
    } else {
        None
    };
}

pub fn error(message: impl AsRef<str>) {
    write(1, "ERROR", message.as_ref());
}

pub fn info(message: impl AsRef<str>) {
    write(2, "INFO", message.as_ref());
}

#[allow(dead_code)]
pub fn debug(message: impl AsRef<str>) {
    write(3, "DEBUG", message.as_ref());
}

fn write(level: u8, label: &str, message: &str) {
    let mut guard = LOGGER.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(logger) = guard.as_mut() else {
        return;
    };
    if level > logger.level {
        return;
    }

    if fs::create_dir_all(&logger.dir).is_err() {
        return;
    }
    let path = logger.dir.join("app.log");
    rotate_if_needed(&path);

    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
    let line = log_line(&timestamp.to_string(), label, message);
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = file.write_all(line.as_bytes());
    }
}

/// app.log → app.1.log → app.2.log，最多保留 3 个文件。
fn rotate_if_needed(path: &std::path::Path) {
    let too_large = fs::metadata(path)
        .map(|meta| meta.len() > MAX_LOG_BYTES)
        .unwrap_or(false);
    if !too_large {
        return;
    }
    let dir = match path.parent() {
        Some(dir) => dir,
        None => return,
    };
    let _ = fs::remove_file(dir.join("app.2.log"));
    let _ = fs::rename(dir.join("app.1.log"), dir.join("app.2.log"));
    let _ = fs::rename(path, dir.join("app.1.log"));
}
