//! Routes the SDK's `log` records to a foreign listener.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, OnceLock};

/// Log severity, most severe first.
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum LogLevel {
    Error = 1,
    Warn = 2,
    Info = 3,
    Debug = 4,
    Trace = 5,
}

/// Receives SDK logs, from any thread.
#[uniffi::export(with_foreign)]
pub trait LogListener: Send + Sync {
    fn log(&self, level: LogLevel, message: String);
}

struct Logger(Arc<dyn LogListener>);

impl log::Log for Logger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &log::Record<'_>) {
        let level = match record.level() {
            log::Level::Error => LogLevel::Error,
            log::Level::Warn => LogLevel::Warn,
            log::Level::Info => LogLevel::Info,
            log::Level::Debug => LogLevel::Debug,
            log::Level::Trace => LogLevel::Trace,
        };
        // Nothing to report a failing log listener to.
        let _ = catch_unwind(AssertUnwindSafe(|| {
            self.0.log(level, record.args().to_string())
        }));
    }

    fn flush(&self) {}
}

/// Routes SDK logs to `listener` up to `max_level`. Only the first call per process takes effect.
#[uniffi::export]
pub fn set_logger(listener: Arc<dyn LogListener>, max_level: LogLevel) {
    static LOGGER: OnceLock<Logger> = OnceLock::new();

    if log::set_logger(LOGGER.get_or_init(|| Logger(listener))).is_ok() {
        log::set_max_level(match max_level {
            LogLevel::Error => log::LevelFilter::Error,
            LogLevel::Warn => log::LevelFilter::Warn,
            LogLevel::Info => log::LevelFilter::Info,
            LogLevel::Debug => log::LevelFilter::Debug,
            LogLevel::Trace => log::LevelFilter::Trace,
        });
    }
}
