//! WebSDR – browserbasierter Empfänger für RTL-SDR-Sticks mit SDR#-artiger Bedienung.

mod demod;
mod dsp;
mod engine;
mod source;
mod state;
mod ws;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use axum::body::Bytes;
use axum::extract::{State, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use clap::Parser;
use tower_http::services::{ServeDir, ServeFile};

use crate::source::{SourceSpec, SourceStatus};
use crate::state::Hub;

#[derive(Parser)]
#[command(version, about = "Browserbasierter SDR-Empfänger für RTL-SDR-Sticks")]
struct Args {
    /// Adresse und Port des Webservers
    #[arg(long, env = "WEBSDR_LISTEN", default_value = "0.0.0.0:8073")]
    listen: SocketAddr,
    /// Signalquelle: usb, usb:N, rtl_tcp://host:port oder sim
    #[arg(long, env = "SDR_SOURCE", default_value = "usb")]
    source: String,
    /// Pfad zur librtlsdr (Standard: Systemsuche)
    #[arg(long, env = "WEBSDR_LIBRTLSDR")]
    librtlsdr: Option<String>,
    /// Ordner mit dem gebauten Frontend
    #[arg(long, env = "WEBSDR_STATIC", default_value = "./static")]
    static_dir: PathBuf,
    /// Ordner für Einstellungen und Lesezeichen
    #[arg(long, env = "WEBSDR_DATA", default_value = "./data")]
    data_dir: PathBuf,
    /// Optionales Passwort für das Steuerrecht
    #[arg(long, env = "CONTROL_PASSWORD")]
    password: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "websdr=info".into()),
        )
        .init();
    let args = Args::parse();
    std::fs::create_dir_all(&args.data_dir)?;

    let spec = SourceSpec::parse(&args.source, args.librtlsdr.clone())?;
    let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded();
    let (iq_tx, iq_rx) = crossbeam_channel::bounded(16);
    let status = Arc::new(parking_lot::Mutex::new(SourceStatus::default()));

    let hub = Arc::new(Hub::new(
        args.data_dir.clone(),
        cmd_tx,
        status.clone(),
        spec.describe(),
        args.password.clone(),
    ));
    let initial = hub.device.lock().clone();
    tracing::info!("Quelle: {}", spec.describe());
    source::spawn(spec, initial, cmd_rx, iq_tx, status);
    engine::spawn(hub.clone(), iq_rx);

    // Hintergrund: Einstellungen speichern, Statusänderungen der Quelle melden.
    {
        let hub = hub.clone();
        tokio::spawn(async move {
            let mut last = String::new();
            let mut tick = tokio::time::interval(Duration::from_secs(1));
            loop {
                tick.tick().await;
                hub.save_if_dirty();
                let s = hub.source_status.lock().clone();
                let now = format!("{}|{}|{}", s.connected, s.message, s.dropped_blocks / 50);
                if now != last {
                    last = now;
                    ws::broadcast_state(&hub);
                }
            }
        });
    }

    let index = args.static_dir.join("index.html");
    let app = Router::new()
        .route("/ws", get(ws_upgrade))
        .route("/health", get(health))
        .route("/api/bookmarks", get(get_bookmarks).put(put_bookmarks))
        .fallback_service(ServeDir::new(&args.static_dir).fallback(ServeFile::new(index)))
        .with_state(hub.clone());

    let listener = tokio::net::TcpListener::bind(args.listen).await?;
    tracing::info!("WebSDR läuft auf http://{}", args.listen);
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    hub.mark_dirty();
    hub.save_if_dirty();
    Ok(())
}

async fn ws_upgrade(ws: WebSocketUpgrade, State(hub): State<Arc<Hub>>) -> Response {
    ws.on_upgrade(move |socket| ws::handle(socket, hub))
}

async fn health(State(hub): State<Arc<Hub>>) -> Response {
    let s = hub.source_status.lock().clone();
    let code = if s.connected { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE };
    (
        code,
        Json(serde_json::json!({
            "ok": s.connected,
            "source": hub.source_desc,
            "message": s.message,
            "device": s.device_name,
            "dropped_blocks": s.dropped_blocks,
        })),
    )
        .into_response()
}

async fn get_bookmarks(State(hub): State<Arc<Hub>>) -> Response {
    match tokio::fs::read(hub.data_dir.join("bookmarks.json")).await {
        Ok(b) => ([("content-type", "application/json")], b).into_response(),
        Err(_) => Json(serde_json::json!([])).into_response(),
    }
}

async fn put_bookmarks(State(hub): State<Arc<Hub>>, body: Bytes) -> Response {
    if body.len() > 1_000_000 {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    }
    if !matches!(serde_json::from_slice::<serde_json::Value>(&body), Ok(v) if v.is_array()) {
        return (StatusCode::BAD_REQUEST, "JSON-Liste erwartet").into_response();
    }
    match tokio::fs::write(hub.data_dir.join("bookmarks.json"), &body).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
