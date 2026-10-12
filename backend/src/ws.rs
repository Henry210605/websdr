//! WebSocket-Protokoll zwischen Browser und Server.
//!
//! Browser → Server: JSON-Befehle (`{"cmd": "...", ...}`).
//! Server → Browser: JSON-Zustand (`{"type": "state", ...}`) und Binärframes
//! (Typ-Byte 1 = Spektrumzeile, 2 = Audio + Signalpegel), siehe engine.rs.

use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::mpsc;

use crate::demod::Mode;
use crate::source::SourceCmd;
use crate::state::{
    follow_center, sanitize_device, usable_range, Client, Hub, FFT_SIZES, FREQ_MAX_HZ,
    FREQ_MIN_HZ, SAMPLE_RATES,
};

#[derive(Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
enum Command {
    /// VFO setzen. Ist `follow` gesetzt (Standard), wird die Mitte bei Bedarf nachgeführt.
    Tune { vfo: u64, #[serde(default = "yes")] follow: bool },
    Center { freq: u64 },
    Mode { mode: Mode, bw: Option<u32> },
    Bandwidth { bw: u32 },
    Gain { db: Option<f32> },
    Samplerate { sps: u32 },
    Ppm { value: i32 },
    Biastee { on: bool },
    FftSize { size: usize },
    View { width: usize, zoom: Option<(f64, f64)> },
    Squelch { db: Option<f32> },
    Audio { on: bool },
    TakeControl { password: Option<String> },
}

fn yes() -> bool {
    true
}

pub async fn handle(socket: WebSocket, hub: Arc<Hub>) {
    let id = hub.next_client_id();
    let (tx, mut rx) = mpsc::channel::<Message>(256);
    let settings = hub.last_client.lock().clone();
    hub.clients.lock().insert(
        id,
        Client { settings, view: Default::default(), audio_on: false, tx: tx.clone() },
    );
    {
        let mut ctl = hub.controller.lock();
        if ctl.is_none() && hub.password.is_none() {
            *ctl = Some(id);
        }
    }
    tracing::info!("Browser {id} verbunden");
    broadcast_state(&hub);

    let (mut sink, mut stream) = socket.split();
    let writer = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if sink.send(msg).await.is_err() {
                break;
            }
        }
    });

    while let Some(Ok(msg)) = stream.next().await {
        match msg {
            Message::Text(t) => match serde_json::from_str::<Command>(&t) {
                Ok(cmd) => apply(&hub, id, cmd),
                Err(e) => {
                    let _ = tx.try_send(error(format!("ungültiger Befehl: {e}")));
                }
            },
            Message::Close(_) => break,
            _ => {}
        }
    }

    hub.clients.lock().remove(&id);
    {
        let mut ctl = hub.controller.lock();
        if *ctl == Some(id) {
            *ctl = if hub.password.is_none() {
                hub.clients.lock().keys().min().copied()
            } else {
                None
            };
        }
    }
    writer.abort();
    tracing::info!("Browser {id} getrennt");
    broadcast_state(&hub);
}

fn error(msg: String) -> Message {
    Message::Text(json!({"type": "error", "message": msg}).to_string().into())
}

fn apply(hub: &Hub, id: u64, cmd: Command) {
    let is_ctl = *hub.controller.lock() == Some(id);
    let deny = |what: &str| {
        if let Some(c) = hub.clients.lock().get(&id) {
            let _ = c.tx.try_send(error(format!("{what}: nur mit Steuerrecht möglich")));
        }
    };
    let mut device_changed = false;

    match cmd {
        Command::Tune { vfo, follow } => {
            let vfo = vfo.clamp(FREQ_MIN_HZ, FREQ_MAX_HZ);
            let (center, rate) = {
                let d = hub.device.lock();
                (d.center_hz, d.sample_rate)
            };
            let mut clients = hub.clients.lock();
            let Some(c) = clients.get_mut(&id) else { return };
            let bw = c.settings.bandwidth_hz;
            let mut new_vfo = vfo;
            if let Some(new_center) = follow_center(vfo, bw, center, rate) {
                if is_ctl && follow {
                    drop(clients);
                    set_center(hub, new_center);
                    device_changed = true;
                    clients = hub.clients.lock();
                } else {
                    // Ohne Steuerrecht nur innerhalb des aktuellen Bandes abstimmen.
                    let (lo, hi) = usable_range(center, rate);
                    new_vfo = vfo.clamp(lo + bw as u64 / 2, hi.saturating_sub(bw as u64 / 2));
                }
            }
            if let Some(c) = clients.get_mut(&id) {
                c.settings.vfo_hz = new_vfo;
            }
        }
        Command::Center { freq } => {
            if !is_ctl {
                return deny("Mittenfrequenz");
            }
            set_center(hub, freq.clamp(FREQ_MIN_HZ, FREQ_MAX_HZ));
            device_changed = true;
        }
        Command::Mode { mode, bw } => {
            let mut clients = hub.clients.lock();
            if let Some(c) = clients.get_mut(&id) {
                let (lo, hi) = mode.bandwidth_limits();
                c.settings.mode = mode;
                c.settings.bandwidth_hz = bw.unwrap_or(mode.default_bandwidth()).clamp(lo, hi);
            }
        }
        Command::Bandwidth { bw } => {
            let mut clients = hub.clients.lock();
            if let Some(c) = clients.get_mut(&id) {
                let (lo, hi) = c.settings.mode.bandwidth_limits();
                c.settings.bandwidth_hz = bw.clamp(lo, hi);
            }
        }
        Command::Squelch { db } => {
            if let Some(c) = hub.clients.lock().get_mut(&id) {
                c.settings.squelch_db = db;
            }
        }
        Command::Audio { on } => {
            if let Some(c) = hub.clients.lock().get_mut(&id) {
                c.audio_on = on;
            }
        }
        Command::View { width, zoom } => {
            if let Some(c) = hub.clients.lock().get_mut(&id) {
                c.view.width = width.clamp(64, 8192);
                if let Some((a, b)) = zoom {
                    let a = a.clamp(0.0, 1.0);
                    let b = b.clamp(0.0, 1.0);
                    if b - a > 1e-4 {
                        c.view.zoom = (a, b);
                    }
                }
            }
            return; // keine Zustandsmeldung nötig
        }
        Command::Gain { db } => {
            if !is_ctl {
                return deny("Verstärkung");
            }
            hub.device.lock().gain_db = db;
            let _ = hub.source_tx.send(SourceCmd::Gain(db));
            device_changed = true;
        }
        Command::Samplerate { sps } => {
            if !is_ctl {
                return deny("Abtastrate");
            }
            if !SAMPLE_RATES.contains(&sps) {
                return;
            }
            hub.device.lock().sample_rate = sps;
            let _ = hub.source_tx.send(SourceCmd::SampleRate(sps));
            device_changed = true;
        }
        Command::Ppm { value } => {
            if !is_ctl {
                return deny("PPM");
            }
            let mut d = hub.device.lock();
            d.ppm = value;
            sanitize_device(&mut d);
            let _ = hub.source_tx.send(SourceCmd::Ppm(d.ppm));
            device_changed = true;
        }
        Command::Biastee { on } => {
            if !is_ctl {
                return deny("Bias-Tee");
            }
            hub.device.lock().bias_tee = on;
            let _ = hub.source_tx.send(SourceCmd::BiasTee(on));
            device_changed = true;
        }
        Command::FftSize { size } => {
            if !is_ctl {
                return deny("FFT-Größe");
            }
            if FFT_SIZES.contains(&size) {
                hub.device.lock().fft_size = size;
                device_changed = true;
            }
        }
        Command::TakeControl { password } => {
            let ok = match &hub.password {
                None => true,
                Some(p) => password.as_deref() == Some(p.as_str()),
            };
            if !ok {
                if let Some(c) = hub.clients.lock().get(&id) {
                    let _ = c.tx.try_send(error("falsches Passwort".into()));
                }
                return;
            }
            *hub.controller.lock() = Some(id);
            device_changed = true;
        }
    }

    // Die Einstellungen des steuernden Browsers sind die, mit denen der nächste Start beginnt.
    if *hub.controller.lock() == Some(id) {
        if let Some(c) = hub.clients.lock().get(&id) {
            *hub.last_client.lock() = c.settings.clone();
        }
        hub.mark_dirty();
    }
    if device_changed {
        broadcast_state(hub);
    } else {
        send_state(hub, id);
    }
}

fn set_center(hub: &Hub, freq: u64) {
    hub.device.lock().center_hz = freq;
    let _ = hub.source_tx.send(SourceCmd::CenterFreq(freq));
}

fn state_json(hub: &Hub, id: u64) -> Option<Message> {
    let device = hub.device.lock().clone();
    let status = hub.source_status.lock().clone();
    let controller = *hub.controller.lock();
    let clients = hub.clients.lock();
    let c = clients.get(&id)?;
    let v = json!({
        "type": "state",
        "device": device,
        "source": {
            "spec": hub.source_desc,
            "connected": status.connected,
            "message": status.message,
            "name": status.device_name,
            "gains": status.gains,
            "dropped_blocks": status.dropped_blocks,
        },
        "you": {
            "id": id,
            "controller": controller == Some(id),
            "settings": c.settings,
            "audio_on": c.audio_on,
        },
        "control_taken": controller.is_some(),
        "password_required": hub.password.is_some(),
        "clients": clients.len(),
        "sample_rates": SAMPLE_RATES,
        "fft_sizes": FFT_SIZES,
    });
    Some(Message::Text(v.to_string().into()))
}

pub fn send_state(hub: &Hub, id: u64) {
    if let Some(msg) = state_json(hub, id) {
        if let Some(c) = hub.clients.lock().get(&id) {
            let _ = c.tx.try_send(msg);
        }
    }
}

pub fn broadcast_state(hub: &Hub) {
    let ids: Vec<u64> = hub.clients.lock().keys().copied().collect();
    for id in ids {
        send_state(hub, id);
    }
}
