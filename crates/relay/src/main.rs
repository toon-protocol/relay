//! The `relay` binary: read the environment, bind, serve until told to stop.
//!
//! A failure to start is one `Error: …` line on stderr and a non-zero exit,
//! which is what the TypeScript relay does and what operators grep for (#185,
//! story 48).

use std::process::ExitCode;

use relay::{Config, RelayError, write_router};
use tokio::net::TcpListener;
use tokio::signal::unix::{SignalKind, signal};

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Error: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), RelayError> {
    let config = Config::from_env(|name| std::env::var(name).ok())?;
    let (host, port) = (config.write_host.as_str(), config.write_port);
    let listener = TcpListener::bind((host, port))
        .await
        .map_err(|source| RelayError::Bind {
            host: host.to_string(),
            port,
            source,
        })?;
    println!(
        "relay {} listening on {host}:{port} (GET /health)",
        env!("CARGO_PKG_VERSION")
    );
    axum::serve(listener, write_router(&config))
        .with_graceful_shutdown(stop_requested())
        .await
        .map_err(RelayError::Serve)
}

/// Resolves on SIGTERM or SIGINT. The relay is PID 1 in its container, where
/// a signal with no handler is ignored, so without this `docker stop` would
/// wait out its grace period and then kill it.
async fn stop_requested() {
    let (Ok(mut terminate), Ok(mut interrupt)) = (
        signal(SignalKind::terminate()),
        signal(SignalKind::interrupt()),
    ) else {
        // No handler could be installed: keep serving and leave stopping to
        // the kill that follows the grace period.
        return std::future::pending().await;
    };
    tokio::select! {
        _ = terminate.recv() => {}
        _ = interrupt.recv() => {}
    }
}
