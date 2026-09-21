mod logging;
mod port;

use std::{error::Error, net::SocketAddr};

use tokio::net::TcpListener;
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    logging::init();

    let bind_addr = SocketAddr::from(([0, 0, 0, 0], port::from_env()?));
    let listener = TcpListener::bind(bind_addr).await?;

    let root = std::path::PathBuf::from(
        std::env::var_os("PLATES_DIR").unwrap_or_else(|| "data/plates".into()),
    );
    let plates = orca_server::plates::Store::open(&root)?;
    let source = match std::env::var("SCAD_LIVE_URL") {
        Ok(url) => Some(orca_server::scad::Source::new(&url)?),
        Err(std::env::VarError::NotPresent) => None,
        Err(std::env::VarError::NotUnicode(_)) => return Err("SCAD_LIVE_URL must be UTF-8".into()),
    };
    let slicer = if let Some(appdir) = std::env::var_os("ORCA_APPDIR") {
        let seconds = match std::env::var("ORCA_TIMEOUT_SECS") {
            Ok(value) => value.parse::<u64>()?,
            Err(std::env::VarError::NotPresent) => 300,
            Err(error) => return Err(error.into()),
        };
        Some(
            orca_server::slicer::Slicer::new(
                appdir.into(),
                std::time::Duration::from_secs(seconds),
            )
            .await?,
        )
    } else {
        None
    };
    let registry = orca_server::registry::router(&root, plates, slicer, source)?;
    info!(%bind_addr, "server listening");
    axum::serve(listener, orca_server::with_mcp(registry))
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    info!("server stopped");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl-C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }

    info!("shutdown signal received");
}
