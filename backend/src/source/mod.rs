//! Signalquellen: echter Stick per USB (librtlsdr), rtl_tcp-Server oder Simulator.
//!
//! Jede Quelle läuft in einem eigenen Thread, besitzt das Gerät exklusiv und
//! wendet Steuerbefehle zwischen zwei Lesevorgängen an. IQ-Blöcke tragen die
//! Mittenfrequenz und Abtastrate, mit der sie aufgenommen wurden, damit die
//! DSP-Kette nach einem Umstimmen sofort konsistent rechnet.

pub mod rtltcp;
pub mod sim;
pub mod usb;

use std::sync::Arc;

use anyhow::{bail, Result};
use crossbeam_channel::{Receiver, Sender, TrySendError};
use num_complex::Complex32;
use parking_lot::Mutex;
use serde::Serialize;

use crate::state::DeviceSettings;

/// Gain-Stufen des R820T/R828D in Zehntel-dB (identisch zu librtlsdr).
pub const R82XX_GAINS: [i32; 29] = [
    0, 9, 14, 27, 37, 77, 87, 125, 144, 157, 166, 197, 207, 229, 254, 280, 297, 328, 338, 364, 372,
    386, 402, 421, 434, 439, 445, 480, 496,
];

/// Samples pro Block (2 Bytes je Sample). 32768 Samples sind bei 2,4 MS/s ca. 14 ms.
pub const BLOCK_SAMPLES: usize = 32768;

/// Anzahl Blöcke, die nach einem Umstimmen verworfen werden (Einschwingen, alte Daten im USB-Puffer).
pub const DISCARD_AFTER_RETUNE: usize = 2;

pub struct IqBlock {
    pub center_hz: u64,
    pub sample_rate: u32,
    pub samples: Vec<Complex32>,
}

#[derive(Debug, Clone)]
pub enum SourceCmd {
    CenterFreq(u64),
    SampleRate(u32),
    Gain(Option<f32>),
    Ppm(i32),
    BiasTee(bool),
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct SourceStatus {
    pub connected: bool,
    pub message: String,
    pub device_name: String,
    /// Verfügbare Verstärkungsstufen in dB.
    pub gains: Vec<f32>,
    pub dropped_blocks: u64,
}

pub type SharedStatus = Arc<Mutex<SourceStatus>>;

#[derive(Debug, Clone)]
pub enum SourceSpec {
    Usb { index: u32, lib: Option<String> },
    RtlTcp { addr: String },
    Sim,
}

impl SourceSpec {
    pub fn parse(s: &str, lib: Option<String>) -> Result<Self> {
        let s = s.trim();
        if s == "sim" {
            return Ok(Self::Sim);
        }
        if s == "usb" {
            return Ok(Self::Usb { index: 0, lib });
        }
        if let Some(i) = s.strip_prefix("usb:") {
            return Ok(Self::Usb { index: i.parse()?, lib });
        }
        if let Some(a) = s.strip_prefix("rtl_tcp://") {
            let addr = if a.contains(':') { a.to_string() } else { format!("{a}:1234") };
            return Ok(Self::RtlTcp { addr });
        }
        bail!("unbekannte Quelle '{s}' (erlaubt: usb, usb:N, rtl_tcp://host:port, sim)")
    }

    pub fn describe(&self) -> String {
        match self {
            Self::Usb { index, .. } => format!("usb:{index}"),
            Self::RtlTcp { addr } => format!("rtl_tcp://{addr}"),
            Self::Sim => "sim".into(),
        }
    }
}

/// Startet den Quellen-Thread.
pub fn spawn(
    spec: SourceSpec,
    initial: DeviceSettings,
    cmds: Receiver<SourceCmd>,
    out: Sender<IqBlock>,
    status: SharedStatus,
) {
    std::thread::Builder::new()
        .name("sdr-source".into())
        .spawn(move || match spec {
            SourceSpec::Usb { index, lib } => usb::run(index, lib, initial, cmds, out, status),
            SourceSpec::RtlTcp { addr } => rtltcp::run(addr, initial, cmds, out, status),
            SourceSpec::Sim => sim::run(initial, cmds, out, status),
        })
        .expect("Quellen-Thread konnte nicht gestartet werden");
}

/// Wandelt rohe u8-IQ-Daten des RTL2832 in komplexe Floats (-1..1) um.
pub fn convert_u8(buf: &[u8], out: &mut Vec<Complex32>) {
    out.clear();
    out.reserve(buf.len() / 2);
    for p in buf.chunks_exact(2) {
        out.push(Complex32::new(
            (p[0] as f32 - 127.5) / 127.5,
            (p[1] as f32 - 127.5) / 127.5,
        ));
    }
}

/// Gibt einen Block an die DSP weiter; ist sie zu langsam, wird der Block verworfen
/// und gezählt (lieber ein kurzer Aussetzer als wachsende Latenz).
pub fn push_block(out: &Sender<IqBlock>, block: IqBlock, status: &SharedStatus) -> bool {
    match out.try_send(block) {
        Ok(()) => true,
        Err(TrySendError::Full(_)) => {
            status.lock().dropped_blocks += 1;
            true
        }
        Err(TrySendError::Disconnected(_)) => false,
    }
}

/// Wählt aus den Gain-Stufen die nächstliegende (Zehntel-dB).
pub fn nearest_gain(gains: &[i32], db: f32) -> i32 {
    let target = (db * 10.0).round() as i32;
    *gains
        .iter()
        .min_by_key(|g| (**g - target).abs())
        .unwrap_or(&target)
}

pub fn set_status(status: &SharedStatus, connected: bool, msg: impl Into<String>) {
    let mut s = status.lock();
    s.connected = connected;
    s.message = msg.into();
}
