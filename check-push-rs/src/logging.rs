use crate::config::Config;
use std::io;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::{
    fmt::{format::Writer, time::FormatTime, FmtContext, FormatEvent, FormatFields},
    registry::LookupSpan,
    EnvFilter,
};

pub fn init(config: &Config) {
    // If verbosity is 0, redirect to /dev/null (silent mode)
    if config.verbosity == 0 {
        return; // Will be handled by main.rs redirecting stdout
    }

    let filter = match config.verbosity {
        1 => EnvFilter::new("info"),
        2 => EnvFilter::new("debug"),
        _ => EnvFilter::new("info"),
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_timer(CustomTimer)
        .with_target(false)
        .with_ansi(false)
        .event_format(CustomFormatter)
        .init();
}

struct CustomTimer;

impl FormatTime for CustomTimer {
    fn format_time(&self, w: &mut Writer<'_>) -> std::fmt::Result {
        let now = chrono::Local::now();
        write!(w, "{}", now.format("%m-%d %H:%M:%S>"))
    }
}

struct CustomFormatter;

impl<S, N> FormatEvent<S, N> for CustomFormatter
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> std::fmt::Result {
        // Format: MM-DD HH:MM:SS> message
        let metadata = event.metadata();
        let level = *metadata.level();

        // Map tracing levels to our verbosity levels
        let should_log = match level {
            Level::ERROR | Level::WARN | Level::INFO => true,
            Level::DEBUG | Level::TRACE => false,
        };

        if !should_log {
            return Ok(());
        }

        // Write timestamp (handled by timer)
        // Write message
        ctx.format_fields(writer.by_ref(), event)?;
        writeln!(writer)
    }
}

// Helper macros for logging that match bash script behavior
#[macro_export]
macro_rules! mustsay {
    ($($arg:tt)*) => {
        eprintln!($($arg)*);
    };
}

#[macro_export]
macro_rules! say {
    ($($arg:tt)*) => {
        tracing::info!($($arg)*);
    };
}

#[macro_export]
macro_rules! verbose {
    ($($arg:tt)*) => {
        tracing::debug!($($arg)*);
    };
}
