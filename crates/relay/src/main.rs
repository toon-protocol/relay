//! The `relay` binary: read the environment, bind, serve until told to stop.
//!
//! A failure to start is one `Error: …` line on stderr and a non-zero exit,
//! which is what the TypeScript relay does and what operators grep for (#185,
//! story 48).

use std::net::SocketAddr;
use std::process::ExitCode;

use relay::{Config, Relay, RelayError};
use tokio::net::TcpListener;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::watch;

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
    let relay = Relay::open(&config)?;
    match &config.connector {
        Some(connector) => println!(
            "[relay] paid write edge: asking {} about {}",
            connector.url, connector.ilp_address
        ),
        None => println!(
            "[relay] paid write edge: none published — set TOON_CONNECTOR_URL and \
             TOON_WRITE_ILP_ADDRESS for a client to be able to pay this relay from its URL alone"
        ),
    }
    // Asked in the background and never awaited: see `connector`.
    let _watch = relay.watch_connector(&config);
    let write = listen(&config.write_host, config.write_port).await?;
    let read = listen(&config.read_host, config.read_port).await?;
    println!(
        "relay {} listening: writes on {}:{} (POST /write, GET /health), reads on {}:{} (NIP-01 WebSocket)",
        env!("CARGO_PKG_VERSION"),
        config.write_host,
        config.write_port,
        config.read_host,
        config.read_port,
    );

    // One stop signal for both listeners; either failing stops the relay.
    let (stop, stopped) = watch::channel(());
    tokio::spawn(async move {
        stop_requested().await;
        drop(stop);
    });
    let until_stopped = |mut stopped: watch::Receiver<()>| async move {
        // The sender is dropped, never sent on: `changed` ends when it is.
        let _ = stopped.changed().await;
    };
    let write = axum::serve(write, relay.write_router())
        .with_graceful_shutdown(until_stopped(stopped.clone()));
    let read = axum::serve(
        read,
        relay
            .read_router()
            .into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(until_stopped(stopped));
    tokio::try_join!(write.into_future(), read.into_future())
        .map(|_| ())
        .map_err(RelayError::Serve)
}

async fn listen(host: &str, port: u16) -> Result<TcpListener, RelayError> {
    TcpListener::bind((host, port))
        .await
        .map_err(|source| RelayError::Bind {
            host: host.to_string(),
            port,
            source,
        })
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
