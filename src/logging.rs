use std::fs;
use std::path::PathBuf;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

/// Maximum age of log files before cleanup (7 days)
const MAX_LOG_AGE_SECS: u64 = 7 * 24 * 60 * 60;

/// Returns the log directory: ~/.local/state/tmuxcc/
fn log_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".local").join("state").join("tmuxcc"))
}

/// Initialize logging. Always writes to a timestamped log file.
/// Default level is INFO; `debug` flag bumps to DEBUG.
pub fn init(debug: bool) {
    let Some(dir) = log_dir() else {
        return;
    };
    if fs::create_dir_all(&dir).is_err() {
        return;
    }

    // Clean up old logs
    cleanup_old_logs(&dir);

    // Create timestamped log file
    let timestamp = chrono::Local::now().format("%Y-%m-%dT%H-%M-%S");
    let log_name = format!("tmuxcc-{}.log", timestamp);
    let log_path = dir.join(&log_name);

    let Ok(log_file) = fs::File::create(&log_path) else {
        return;
    };

    // Update latest.log symlink
    let symlink_path = dir.join("latest.log");
    let _ = fs::remove_file(&symlink_path);
    #[cfg(unix)]
    {
        let _ = std::os::unix::fs::symlink(&log_path, &symlink_path);
    }

    let level = if debug {
        tracing_subscriber::filter::LevelFilter::DEBUG
    } else {
        tracing_subscriber::filter::LevelFilter::INFO
    };

    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(log_file)
        .with_ansi(false)
        .with_target(true);

    tracing_subscriber::registry()
        .with(file_layer)
        .with(level)
        .init();

    tracing::info!(log_path = %log_path.display(), level = %level, "logging initialized");
}

/// Remove log files older than MAX_LOG_AGE_SECS
fn cleanup_old_logs(dir: &PathBuf) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let now = std::time::SystemTime::now();

    for entry in entries.flatten() {
        let path = entry.path();
        // Only clean up tmuxcc-*.log files
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !name.starts_with("tmuxcc-") || !name.ends_with(".log") {
            continue;
        }
        if let Ok(metadata) = path.metadata() {
            if let Ok(modified) = metadata.modified() {
                if let Ok(age) = now.duration_since(modified) {
                    if age.as_secs() > MAX_LOG_AGE_SECS {
                        let _ = fs::remove_file(&path);
                    }
                }
            }
        }
    }
}

/// Returns the path to the latest log file (for display to user)
pub fn latest_log_path() -> Option<PathBuf> {
    log_dir().map(|d| d.join("latest.log"))
}
