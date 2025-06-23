use anyhow::Result;
use std::{fs, io, path::Path};
use tracing_appender::non_blocking;
use tracing_subscriber::{filter::LevelFilter, fmt, prelude::*, EnvFilter};

pub struct LogGuards {
    guard1: non_blocking::WorkerGuard,
    guard2: non_blocking::WorkerGuard,
}

pub fn setup_logging(bin_dir: impl AsRef<Path>) -> Result<LogGuards> {
    let f = io::BufWriter::new(fs::File::create(bin_dir.as_ref().join("dll_hook.log"))?);
    let (file_writer, guard1) = non_blocking(f);

    let (console_writer, guard2) = non_blocking(io::stdout());

    let file_layer = fmt::layer()
        .with_writer(file_writer)
        .with_ansi(false)
        .with_target(true)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true)
        .with_filter(LevelFilter::DEBUG);

    let console_layer = fmt::layer()
        .with_writer(console_writer)
        .with_ansi(true)
        .compact()
        .with_filter(LevelFilter::INFO);

    tracing_subscriber::registry()
        .with(file_layer)
        .with(console_layer)
        .with(EnvFilter::from_default_env())
        .init();

    tracing::info!("Logging initialized");

    Ok(LogGuards { guard1, guard2 })
}
