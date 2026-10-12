//! Quelle über einen rtl_tcp-Server (z. B. rtl_tcp.exe nativ unter Windows).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use crossbeam_channel::{Receiver, Sender};

use super::{
    convert_u8, nearest_gain, push_block, set_status, IqBlock, SharedStatus, SourceCmd,
    BLOCK_SAMPLES, DISCARD_AFTER_RETUNE, R82XX_GAINS,
};
use crate::state::DeviceSettings;

fn send(w: &mut TcpStream, cmd: u8, param: u32) -> std::io::Result<()> {
    let mut b = [0u8; 5];
    b[0] = cmd;
    b[1..].copy_from_slice(&param.to_be_bytes());
    w.write_all(&b)
}

fn apply(w: &mut TcpStream, cmd: &SourceCmd) -> std::io::Result<()> {
    match *cmd {
        SourceCmd::CenterFreq(hz) => send(w, 0x01, hz.min(u32::MAX as u64) as u32),
        SourceCmd::SampleRate(sps) => send(w, 0x02, sps),
        SourceCmd::Gain(None) => send(w, 0x03, 0),
        SourceCmd::Gain(Some(db)) => {
            send(w, 0x03, 1)?;
            send(w, 0x04, nearest_gain(&R82XX_GAINS, db) as u32)
        }
        SourceCmd::Ppm(ppm) => send(w, 0x05, ppm as u32),
        SourceCmd::BiasTee(on) => send(w, 0x0e, on as u32),
    }
}

pub fn run(
    addr: String,
    mut settings: DeviceSettings,
    cmds: Receiver<SourceCmd>,
    out: Sender<IqBlock>,
    status: SharedStatus,
) {
    status.lock().gains = R82XX_GAINS.iter().map(|g| *g as f32 / 10.0).collect();
    loop {
        match session(&addr, &mut settings, &cmds, &out, &status) {
            Ok(()) => return,
            Err(e) => {
                tracing::warn!("rtl_tcp {addr}: {e:#}");
                set_status(&status, false, format!("rtl_tcp {addr}: {e:#}"));
                std::thread::sleep(Duration::from_secs(2));
                while let Ok(cmd) = cmds.try_recv() {
                    update(&mut settings, &cmd);
                }
            }
        }
    }
}

fn update(s: &mut DeviceSettings, cmd: &SourceCmd) {
    match *cmd {
        SourceCmd::CenterFreq(hz) => s.center_hz = hz,
        SourceCmd::SampleRate(r) => s.sample_rate = r,
        SourceCmd::Gain(g) => s.gain_db = g,
        SourceCmd::Ppm(p) => s.ppm = p,
        SourceCmd::BiasTee(b) => s.bias_tee = b,
    }
}

fn session(
    addr: &str,
    settings: &mut DeviceSettings,
    cmds: &Receiver<SourceCmd>,
    out: &Sender<IqBlock>,
    status: &SharedStatus,
) -> Result<()> {
    let sock_addr = std::net::ToSocketAddrs::to_socket_addrs(addr)
        .context("Adresse nicht auflösbar")?
        .next()
        .ok_or_else(|| anyhow!("Adresse nicht auflösbar"))?;
    let mut stream = TcpStream::connect_timeout(&sock_addr, Duration::from_secs(5))?;
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut header = [0u8; 12];
    stream.read_exact(&mut header)?;
    if &header[0..4] != b"RTL0" {
        return Err(anyhow!("kein rtl_tcp-Server (Kennung fehlt)"));
    }
    let tuner = u32::from_be_bytes(header[4..8].try_into().unwrap());
    let mut writer = stream.try_clone()?;
    for cmd in [
        SourceCmd::SampleRate(settings.sample_rate),
        SourceCmd::CenterFreq(settings.center_hz),
        SourceCmd::Ppm(settings.ppm),
        SourceCmd::Gain(settings.gain_db),
        SourceCmd::BiasTee(settings.bias_tee),
    ] {
        apply(&mut writer, &cmd)?;
    }
    status.lock().device_name = format!("rtl_tcp {addr} (Tuner-Typ {tuner})");
    set_status(status, true, format!("verbunden mit rtl_tcp {addr}"));

    let mut raw = vec![0u8; BLOCK_SAMPLES * 2];
    let mut discard = DISCARD_AFTER_RETUNE;
    loop {
        while let Ok(cmd) = cmds.try_recv() {
            apply(&mut writer, &cmd)?;
            update(settings, &cmd);
            if matches!(cmd, SourceCmd::CenterFreq(_) | SourceCmd::SampleRate(_)) {
                // Über TCP kommen noch Daten mit der alten Frequenz; großzügiger verwerfen.
                discard = DISCARD_AFTER_RETUNE + 2;
            }
        }
        stream.read_exact(&mut raw)?;
        if discard > 0 {
            discard -= 1;
            continue;
        }
        let mut samples = Vec::new();
        convert_u8(&raw, &mut samples);
        let block = IqBlock {
            center_hz: settings.center_hz,
            sample_rate: settings.sample_rate,
            samples,
        };
        if !push_block(out, block, status) {
            return Ok(());
        }
    }
}
