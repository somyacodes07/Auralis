//! Diagnostics log and crash capture.
//!
//! A small, std-only file logger for the long-running desktop processes (`gui`, `play`).
//! Each entry is one line: an RFC 3339 UTC timestamp, a level, and the message. The log
//! rotates to `auralis.log.1` once it would exceed [`MAX_LOG_BYTES`], keeping one old file.
//!
//! Logging never panics and never reports its own failures: a diagnostics problem must not
//! take the wallpaper down. Before [`init`] runs, log calls are silently ignored.
//!
//! Privacy (`AGENTS.md` Principle 8): callers log media by file name only, using
//! [`file_name_only`] and [`redact_path`], never the full path (which contains the user's
//! account name and folder layout).

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// Rotate the log once it would grow past this many bytes.
pub const MAX_LOG_BYTES: u64 = 1024 * 1024;

const LOG_FILE_NAME: &str = "auralis.log";

/// Severity of a log entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

impl Level {
    const fn label(self) -> &'static str {
        match self {
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }
}

/// Append-only log file with single-generation size rotation.
#[derive(Debug)]
struct LogFile {
    path: PathBuf,
    max_bytes: u64,
}

impl LogFile {
    fn rotated_path(&self) -> PathBuf {
        let mut name = self.path.as_os_str().to_os_string();
        name.push(".1");
        PathBuf::from(name)
    }

    /// Append one line, rotating first if the line would push the file past `max_bytes`.
    /// All I/O errors are ignored on purpose.
    fn append(&self, line: &str) {
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let current = fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0);
        if current > 0 && current + line.len() as u64 > self.max_bytes {
            let rotated = self.rotated_path();
            // `rename` replaces an existing target on Unix; remove it first for Windows.
            let _ = fs::remove_file(&rotated);
            let _ = fs::rename(&self.path, &rotated);
        }
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            let _ = file.write_all(line.as_bytes());
        }
    }
}

static LOGGER: OnceLock<Mutex<LogFile>> = OnceLock::new();

/// Platform-standard log file location.
///
/// - Windows: `%LOCALAPPDATA%\Auralis\logs\auralis.log`
/// - macOS: `~/Library/Logs/Auralis/auralis.log`
/// - Other: a `logs` folder next to the configuration file.
#[must_use]
pub fn default_log_path() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            if !local.is_empty() {
                return PathBuf::from(local)
                    .join("Auralis")
                    .join("logs")
                    .join(LOG_FILE_NAME);
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = std::env::var_os("HOME") {
            if !home.is_empty() {
                return PathBuf::from(home)
                    .join("Library")
                    .join("Logs")
                    .join("Auralis")
                    .join(LOG_FILE_NAME);
            }
        }
    }
    let config_path = crate::config::AppConfig::default_config_path();
    config_path
        .parent()
        .map_or_else(PathBuf::new, Path::to_path_buf)
        .join("logs")
        .join(LOG_FILE_NAME)
}

/// Start file logging at [`default_log_path`] and install the panic hook.
///
/// Safe to call more than once; only the first call takes effect. Returns the log path.
pub fn init() -> PathBuf {
    let path = default_log_path();
    init_at(path.clone(), MAX_LOG_BYTES);
    path
}

fn init_at(path: PathBuf, max_bytes: u64) {
    let mut first = false;
    LOGGER.get_or_init(|| {
        first = true;
        Mutex::new(LogFile { path, max_bytes })
    });
    if first {
        install_panic_hook();
    }
}

/// Record panics in the log, then run the previous hook so the usual stderr report still
/// appears. Release builds use `panic = "abort"`; the hook runs before the abort.
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info.payload();
        let message = payload
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "non-string panic payload".to_string());
        let location = info.location().map_or_else(
            || "unknown location".to_string(),
            |l| format!("{}:{}:{}", l.file(), l.line(), l.column()),
        );
        let thread = std::thread::current();
        let thread_name = thread.name().unwrap_or("unnamed");
        log(
            Level::Error,
            &format!("panic on thread '{thread_name}' at {location}: {message}"),
        );
        previous(info);
    }));
}

/// Write one entry. Does nothing before [`init`] and never fails visibly.
pub fn log(level: Level, message: &str) {
    let Some(logger) = LOGGER.get() else {
        return;
    };
    let line = format_line(SystemTime::now(), level, message);
    // A poisoned lock only means another thread panicked mid-write; keep logging.
    let guard = logger
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard.append(&line);
}

pub fn info(message: &str) {
    log(Level::Info, message);
}

pub fn warn(message: &str) {
    log(Level::Warn, message);
}

pub fn error(message: &str) {
    log(Level::Error, message);
}

/// The final path component, for logging media without revealing the folder layout.
#[must_use]
pub fn file_name_only(path: &Path) -> String {
    path.file_name().map_or_else(
        || "<unnamed>".to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// Replace every occurrence of `path` in `message` with its file name only.
///
/// Error messages from validation and native playback often embed the full media path.
#[must_use]
pub fn redact_path(message: &str, path: &Path) -> String {
    let full = path.to_string_lossy();
    if full.is_empty() {
        return message.to_string();
    }
    message.replace(full.as_ref(), &file_name_only(path))
}

/// One log line: `2026-11-19T06:57:03.123Z WARN message\n`. Line breaks inside the message
/// are flattened so each entry stays on one line.
fn format_line(time: SystemTime, level: Level, message: &str) -> String {
    let flattened: String = message
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect();
    format!("{} {} {}\n", format_utc(time), level.label(), flattened)
}

/// Format a time as an RFC 3339 UTC timestamp with millisecond precision using only std.
fn format_utc(time: SystemTime) -> String {
    let since_epoch = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let total_secs = since_epoch.as_secs();
    let millis = since_epoch.subsec_millis();
    let days = total_secs / 86_400;
    let secs_of_day = total_secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60
    )
}

/// Convert days since 1970-01-01 to a proleptic Gregorian (year, month, day).
/// Howard Hinnant's `civil_from_days`, restricted to non-negative day counts.
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("auralis_diagnostics_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn line_format_has_utc_timestamp_level_and_single_line_message() {
        let time = UNIX_EPOCH + Duration::from_millis(1_700_000_000_123);
        assert_eq!(
            format_line(time, Level::Warn, "first\r\nsecond"),
            "2023-11-14T22:13:20.123Z WARN first  second\n"
        );
        assert_eq!(
            format_line(UNIX_EPOCH, Level::Error, "x"),
            "1970-01-01T00:00:00.000Z ERROR x\n"
        );
    }

    #[test]
    fn utc_formatting_handles_leap_days_and_year_ends() {
        // 2024-02-29T12:00:00Z and 2026-12-31T23:59:59Z.
        assert_eq!(
            format_utc(UNIX_EPOCH + Duration::from_secs(1_709_208_000)),
            "2024-02-29T12:00:00.000Z"
        );
        assert_eq!(
            format_utc(UNIX_EPOCH + Duration::from_secs(1_798_761_599)),
            "2026-12-31T23:59:59.000Z"
        );
    }

    #[test]
    fn redaction_keeps_only_the_file_name() {
        let path = Path::new("/Users/someone/Movies/Private/ocean.mp4");
        assert_eq!(file_name_only(path), "ocean.mp4");
        let message = format!("Media file not found: {}", path.display());
        let redacted = redact_path(&message, path);
        assert_eq!(redacted, "Media file not found: ocean.mp4");
        assert!(!redacted.contains("someone"));
        assert_eq!(file_name_only(Path::new("")), "<unnamed>");
        assert_eq!(redact_path("unchanged", Path::new("")), "unchanged");
    }

    #[test]
    fn log_rotates_to_one_backup_file() {
        let dir = temp_dir("rotation");
        let log = LogFile {
            path: dir.join(LOG_FILE_NAME),
            max_bytes: 64,
        };
        let line = "0123456789012345678901234567890\n"; // 32 bytes
        log.append(line);
        log.append(line);
        assert_eq!(fs::metadata(&log.path).unwrap().len(), 64);
        assert!(!log.rotated_path().exists());

        log.append("third\n");
        assert_eq!(
            fs::read_to_string(log.rotated_path()).unwrap(),
            line.repeat(2)
        );
        assert_eq!(fs::read_to_string(&log.path).unwrap(), "third\n");

        // A second rotation replaces the old backup instead of accumulating files.
        log.append(&"y".repeat(63));
        assert_eq!(fs::read_to_string(log.rotated_path()).unwrap(), "third\n");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 2);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn panic_hook_records_message_and_location() {
        // The only test that initializes the process-wide logger, so it owns the log path.
        let dir = temp_dir("panic_hook");
        let path = dir.join(LOG_FILE_NAME);
        init_at(path.clone(), MAX_LOG_BYTES);
        let result = std::thread::Builder::new()
            .name("diagnostics-panic-test".into())
            .spawn(|| panic!("diagnostics panic marker"))
            .unwrap()
            .join();
        assert!(result.is_err());
        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("ERROR panic on thread 'diagnostics-panic-test' at src"));
        assert!(contents.contains("diagnostics.rs:"));
        assert!(contents.contains("diagnostics panic marker"));
    }

    #[test]
    fn default_log_path_uses_platform_log_location() {
        let path = default_log_path();
        assert_eq!(path.file_name().unwrap(), LOG_FILE_NAME);
        #[cfg(target_os = "windows")]
        assert!(path.ends_with(Path::new("Auralis").join("logs").join(LOG_FILE_NAME)));
        #[cfg(target_os = "macos")]
        assert!(path.ends_with(
            Path::new("Library")
                .join("Logs")
                .join("Auralis")
                .join(LOG_FILE_NAME)
        ));
    }
}
