mod actor;
mod api;
mod state;
use anyhow::{ensure, Context, Result};
use state::*;
use std::{
    env,
    net::SocketAddr,
    sync::{atomic::AtomicBool, Arc, Mutex},
};
use switch_pro::nfc::Scan;
use tokio::sync::{mpsc, watch};

fn setting(name: &str, default: &str) -> String {
    env::var(name).unwrap_or_else(|_| default.into())
}
#[tokio::main]
async fn main() -> Result<()> {
    let simulation = match env::args().nth(1).as_deref() {
        None => false,
        Some("--simulate") => true,
        _ => anyhow::bail!("Usage: amiibo-pi [--simulate]"),
    };
    let token = env::var("AMIIBO_TOKEN")
        .context("Set AMIIBO_TOKEN to a random token of 24 or more URL-safe characters")?;
    ensure!(
        token.len() >= 24
            && token.len() <= 128
            && token
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
        "Invalid AMIIBO_TOKEN"
    );
    let bind: SocketAddr = setting("AMIIBO_BIND", "127.0.0.1:8787").parse()?;
    let db = amiibo_core::Library::open(setting("AMIIBO_DB", "amiibo.sqlite"))?;
    let web = setting("AMIIBO_WEB", "web/dist");
    ensure!(
        std::path::Path::new(&web).join("index.html").is_file(),
        "Missing compiled web assets in {web}; build the web application first"
    );
    let (tx, rx) = mpsc::channel(16);
    let (version, _) = watch::channel(0);
    let diagnostics = if let Ok(bytes) = std::fs::read("/run/amiibo-bt/diagnostic.json") {
        serde_json::from_slice(&bytes)?
    } else {
        amiibo_bluetooth::diagnostic().await
    };
    let app = Arc::new(App {
        token,
        simulation,
        bind: bind.to_string(),
        db: Mutex::new(db),
        status: Mutex::new(Status {
            link: LinkState::Idle,
            peer: None,
            report: None,
            mtu: None,
            power: 0,
            scan: Scan::Idle,
            armed_name: None,
            error: None,
            simulation,
        }),
        logs: Mutex::new(Default::default()),
        version,
        tx,
        closing: AtomicBool::new(false),
        debug: AtomicBool::new(setting("AMIIBO_LOG", "info") == "debug"),
        diagnostics,
    });
    let listener = tokio::net::TcpListener::bind(bind).await?;
    app.log(format!(
        "Listening at {} ({})",
        listener.local_addr()?,
        if simulation { "Simulation" } else { "Hardware" }
    ));
    let actor = tokio::spawn(actor::run(
        app.clone(),
        rx,
        !simulation
            && setting("AMIIBO_AUTORECONNECT", "false") == "true"
            && std::path::Path::new("/run/amiibo-bt/paired").exists(),
    ));
    let shutdown_app = app.clone();
    axum::serve(listener, api::router(app, web))
        .with_graceful_shutdown(async move {
            shutdown().await;
            shutdown_app
                .closing
                .store(true, std::sync::atomic::Ordering::Relaxed);
            shutdown_app.changed();
            let (tx, rx) = tokio::sync::oneshot::channel();
            if shutdown_app
                .tx
                .send(Request {
                    action: Action::Disconnect,
                    response: tx,
                })
                .await
                .is_ok()
            {
                let _ = rx.await;
            }
            // Wake open event sockets so service shutdown can close them.
            shutdown_app.changed();
        })
        .await?;
    actor.abort();
    let _ = actor.await;
    Ok(())
}
async fn shutdown() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("signal handler");
        tokio::select! {_=tokio::signal::ctrl_c()=>{},_=term.recv()=>{}}
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
