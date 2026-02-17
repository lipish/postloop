use chrono::Local;
use log::{Level, Log, Metadata, Record};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::Mutex;

pub struct PloopLogger {
    file: Mutex<File>,
    level: Level,
}

impl PloopLogger {
    /// Create a new logger instance
    pub fn new(log_file: &str, level: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_file)?;

        let level = match level.to_lowercase().as_str() {
            "trace" => Level::Trace,
            "debug" => Level::Debug,
            "info" => Level::Info,
            "warn" => Level::Warn,
            "error" => Level::Error,
            _ => Level::Info,
        };

        Ok(PloopLogger {
            file: Mutex::new(file),
            level,
        })
    }

    /// Initialize the logger as the global logger
    pub fn init(log_file: &str, level: &str) -> Result<(), Box<dyn std::error::Error>> {
        let logger = PloopLogger::new(log_file, level)?;
        log::set_boxed_logger(Box::new(logger))?;
        log::set_max_level(log::LevelFilter::Trace);
        Ok(())
    }

    /// Log a message with timestamp and commit hash
    #[allow(dead_code)]
    pub fn log_deployment(
        &self,
        commit_hash: &str,
        operation: &str,
        result: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S");
        let message = format!(
            "[{}] Commit: {} | Operation: {} | Result: {}\n",
            timestamp, commit_hash, operation, result
        );

        let mut file = self.file.lock().unwrap();
        file.write_all(message.as_bytes())?;
        file.flush()?;
        Ok(())
    }
}

impl Log for PloopLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S");
            let message = format!(
                "[{}] {} - {}\n",
                timestamp,
                record.level(),
                record.args()
            );

            if let Ok(mut file) = self.file.lock() {
                let _ = file.write_all(message.as_bytes());
                let _ = file.flush();
            }
        }
    }

    fn flush(&self) {
        if let Ok(mut file) = self.file.lock() {
            let _ = file.flush();
        }
    }
}

/// Initialize a simple console logger for development
#[allow(dead_code)]
pub fn init_simple_logger() {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .init();
}
