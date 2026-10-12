//! Gemeinsamer Zustand: Geräteeinstellungen, verbundene Browser, Steuerrecht.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use axum::extract::ws::Message;
use crossbeam_channel::Sender;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::demod::Mode;
use crate::source::{SharedStatus, SourceCmd};

/// Frequenzbereich des RTL-SDR Blog V4 (HF über den eingebauten Upconverter).
pub const FREQ_MIN_HZ: u64 = 100_000;
pub const FREQ_MAX_HZ: u64 = 1_766_000_000;

/// Erlaubte Abtastraten (wie in SDR#).
pub const SAMPLE_RATES: [u32; 10] = [
    250_000, 1_024_000, 1_400_000, 1_800_000, 1_920_000, 2_048_000, 2_400_000, 2_560_000,
    2_880_000, 3_200_000,
];

pub const FFT_SIZES: [usize; 7] = [1024, 2048, 4096, 8192, 16384, 32768, 65536];

/// Anteil der Bandbreite, der als nutzbar gilt (die äußeren Ränder dämpft der RTL-Filter).
pub const USABLE_FRACTION: f64 = 0.9;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DeviceSettings {
    pub center_hz: u64,
    pub sample_rate: u32,
    /// None = automatische Verstärkung (Tuner-AGC).
    pub gain_db: Option<f32>,
    pub ppm: i32,
    pub bias_tee: bool,
    pub fft_size: usize,
}

impl Default for DeviceSettings {
    fn default() -> Self {
        Self {
            center_hz: 99_700_000,
            sample_rate: 2_400_000,
            gain_db: Some(29.7),
            ppm: 0,
            bias_tee: false,
            fft_size: 8192,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ClientSettings {
    pub vfo_hz: u64,
    pub mode: Mode,
    pub bandwidth_hz: u32,
    /// None = Rauschsperre aus.
    pub squelch_db: Option<f32>,
}

impl Default for ClientSettings {
    fn default() -> Self {
        Self {
            vfo_hz: 100_000_000,
            mode: Mode::Wfm,
            bandwidth_hz: Mode::Wfm.default_bandwidth(),
            squelch_db: None,
        }
    }
}

/// Darstellungswünsche eines Browsers für das Spektrum.
#[derive(Debug, Clone)]
pub struct ViewSettings {
    pub width: usize,
    /// Ausschnitt des Bandes als Anteil 0..1 (Zoom).
    pub zoom: (f64, f64),
}

impl Default for ViewSettings {
    fn default() -> Self {
        Self { width: 1024, zoom: (0.0, 1.0) }
    }
}

pub struct Client {
    pub settings: ClientSettings,
    pub view: ViewSettings,
    pub audio_on: bool,
    pub tx: mpsc::Sender<Message>,
}

/// Momentaufnahme eines Clients für den DSP-Thread.
#[derive(Clone)]
pub struct ClientSnapshot {
    pub id: u64,
    pub settings: ClientSettings,
    pub view: ViewSettings,
    pub audio_on: bool,
    pub tx: mpsc::Sender<Message>,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct Persisted {
    device: DeviceSettings,
    client: ClientSettings,
}

pub struct Hub {
    pub device: Mutex<DeviceSettings>,
    /// Einstellungen, mit denen ein neuer Browser startet (die zuletzt gesteuerten).
    pub last_client: Mutex<ClientSettings>,
    pub clients: Mutex<HashMap<u64, Client>>,
    pub controller: Mutex<Option<u64>>,
    pub source_tx: Sender<SourceCmd>,
    pub source_status: SharedStatus,
    pub source_desc: String,
    pub password: Option<String>,
    pub data_dir: PathBuf,
    next_id: AtomicU64,
    dirty: AtomicBool,
}

impl Hub {
    pub fn new(
        data_dir: PathBuf,
        source_tx: Sender<SourceCmd>,
        source_status: SharedStatus,
        source_desc: String,
        password: Option<String>,
    ) -> Self {
        let persisted: Persisted = std::fs::read(data_dir.join("settings.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let mut device = persisted.device;
        sanitize_device(&mut device);
        Self {
            device: Mutex::new(device),
            last_client: Mutex::new(persisted.client),
            clients: Mutex::new(HashMap::new()),
            controller: Mutex::new(None),
            source_tx,
            source_status,
            source_desc,
            password: password.filter(|p| !p.is_empty()),
            data_dir,
            next_id: AtomicU64::new(1),
            dirty: AtomicBool::new(false),
        }
    }

    pub fn next_client_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    pub fn snapshot_clients(&self) -> Vec<ClientSnapshot> {
        self.clients
            .lock()
            .iter()
            .map(|(id, c)| ClientSnapshot {
                id: *id,
                settings: c.settings.clone(),
                view: c.view.clone(),
                audio_on: c.audio_on,
                tx: c.tx.clone(),
            })
            .collect()
    }

    pub fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::Relaxed);
    }

    /// Speichert die Einstellungen, falls sich seit dem letzten Aufruf etwas geändert hat.
    pub fn save_if_dirty(&self) {
        if !self.dirty.swap(false, Ordering::Relaxed) {
            return;
        }
        let p = Persisted {
            device: self.device.lock().clone(),
            client: self.last_client.lock().clone(),
        };
        let path = self.data_dir.join("settings.json");
        let tmp = self.data_dir.join("settings.json.tmp");
        let res = serde_json::to_vec_pretty(&p)
            .map_err(anyhow::Error::from)
            .and_then(|b| std::fs::write(&tmp, b).map_err(Into::into))
            .and_then(|_| std::fs::rename(&tmp, &path).map_err(Into::into));
        if let Err(e) = res {
            tracing::warn!("Einstellungen konnten nicht gespeichert werden: {e}");
        }
    }
}

pub fn sanitize_device(d: &mut DeviceSettings) {
    if !SAMPLE_RATES.contains(&d.sample_rate) {
        d.sample_rate = 2_400_000;
    }
    if !FFT_SIZES.contains(&d.fft_size) {
        d.fft_size = 8192;
    }
    d.center_hz = d.center_hz.clamp(FREQ_MIN_HZ, FREQ_MAX_HZ);
    d.ppm = d.ppm.clamp(-200, 200);
}

/// Nutzbarer Frequenzbereich (untere, obere Grenze) bei gegebener Mitte und Abtastrate.
pub fn usable_range(center: u64, rate: u32) -> (u64, u64) {
    let half = (rate as f64 * USABLE_FRACTION / 2.0) as u64;
    (center.saturating_sub(half), center + half)
}

/// SDR#-artiges Nachführen: Liegt der VFO außerhalb des nutzbaren Bandes, wird eine neue
/// Mitte so gewählt, dass der VFO mit Platz in Bewegungsrichtung im Band liegt.
/// Gibt None zurück, wenn kein Umstimmen nötig ist.
pub fn follow_center(vfo: u64, bw: u32, center: u64, rate: u32) -> Option<u64> {
    let (lo, hi) = usable_range(center, rate);
    let half_bw = bw as u64 / 2;
    if vfo >= lo + half_bw && vfo + half_bw <= hi {
        return None;
    }
    let shift = (rate as f64 * 0.3) as u64;
    let new_center = if vfo > center { vfo + shift } else { vfo.saturating_sub(shift) };
    Some(new_center.clamp(FREQ_MIN_HZ, FREQ_MAX_HZ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follow_center_keeps_inside() {
        assert_eq!(follow_center(100_000_000, 200_000, 100_100_000, 2_400_000), None);
    }

    #[test]
    fn follow_center_moves_up_with_room_ahead() {
        let c = follow_center(102_000_000, 12_500, 100_000_000, 2_400_000).unwrap();
        assert!(c > 102_000_000);
        let (lo, hi) = usable_range(c, 2_400_000);
        assert!(lo < 102_000_000 && hi > 102_000_000);
    }

    #[test]
    fn follow_center_moves_down() {
        let c = follow_center(90_000_000, 12_500, 100_000_000, 2_400_000).unwrap();
        assert!(c < 90_000_000);
    }
}
