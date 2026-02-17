use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use chrono::Local;

// Global log file path
static LOG_FILE: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Initialize logger and create session log file
pub fn init_logger() -> std::io::Result<()> {
    let log_dir = crate::storage::get_app_data_dir()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?
        .join("logs");

    std::fs::create_dir_all(&log_dir)?;

    let session_file = log_dir.join(format!(
        "session-{}.log",
        Local::now().format("%Y%m%d-%H%M%S")
    ));

    // Store globally
    let mut log_path = LOG_FILE.lock().unwrap();
    *log_path = Some(session_file);

    Ok(())
}

/// Log a message to the session file
pub fn log_to_file(message: &str) {
    let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S");
    let log_line = format!("[{}] {}\n", timestamp, message);

    if let Some(path) = LOG_FILE.lock().unwrap().as_ref() {
        let _ = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .and_then(|mut f| f.write_all(log_line.as_bytes()));
    }
}
