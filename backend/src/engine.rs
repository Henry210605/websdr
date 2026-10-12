//! DSP-Thread: berechnet aus den IQ-Blöcken Spektrum und Audio für alle Browser.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::ws::Message;
use crossbeam_channel::Receiver;

use crate::demod::Chain;
use crate::dsp::spectrum::{render, Spectrum};
use crate::source::IqBlock;
use crate::state::Hub;

pub const SPECTRUM_FPS: u32 = 25;

pub const MSG_SPECTRUM: u8 = 1;
pub const MSG_AUDIO: u8 = 2;

pub fn spawn(hub: Arc<Hub>, rx: Receiver<IqBlock>) {
    std::thread::Builder::new()
        .name("dsp".into())
        .spawn(move || run(hub, rx))
        .expect("DSP-Thread konnte nicht gestartet werden");
}

fn run(hub: Arc<Hub>, rx: Receiver<IqBlock>) {
    let mut spectrum: Option<Spectrum> = None;
    let mut chains: HashMap<u64, Chain> = HashMap::new();
    let mut pixels = Vec::new();
    let mut audio = Vec::new();

    while let Ok(block) = rx.recv() {
        let fft_size = hub.device.lock().fft_size;
        if spectrum
            .as_ref()
            .map_or(true, |s| s.size() != fft_size || s.rate() != block.sample_rate)
        {
            spectrum = Some(Spectrum::new(fft_size, block.sample_rate, SPECTRUM_FPS));
        }
        let frame = spectrum.as_mut().unwrap().push(&block.samples, block.center_hz);
        let clients = hub.snapshot_clients();
        chains.retain(|id, _| clients.iter().any(|c| c.id == *id));

        for c in &clients {
            if let Some(f) = &frame {
                let span = f.sample_rate as f64;
                let start = f.center_hz as f64 + (c.view.zoom.0 - 0.5) * span;
                let end = f.center_hz as f64 + (c.view.zoom.1 - 0.5) * span;
                render(&f.db, c.view.zoom, c.view.width, &mut pixels);
                let mut msg = Vec::with_capacity(24 + pixels.len());
                msg.push(MSG_SPECTRUM);
                msg.extend_from_slice(&[0; 7]);
                msg.extend_from_slice(&start.to_le_bytes());
                msg.extend_from_slice(&end.to_le_bytes());
                msg.extend_from_slice(&pixels);
                // Ist der Browser zu langsam, wird die Zeile verworfen statt gepuffert.
                let _ = c.tx.try_send(Message::Binary(msg.into()));
            }

            let s = &c.settings;
            let chain = chains.entry(c.id).or_insert_with(|| {
                Chain::new(s.mode, s.bandwidth_hz, block.sample_rate)
            });
            if !chain.matches(s.mode, s.bandwidth_hz, block.sample_rate) {
                *chain = Chain::new(s.mode, s.bandwidth_hz, block.sample_rate);
            }
            let offset = s.vfo_hz as f64 - block.center_hz as f64;
            audio.clear();
            chain.process(&block.samples, offset, s.squelch_db, &mut audio);

            let mut msg = Vec::with_capacity(8 + audio.len() * 2);
            msg.push(MSG_AUDIO);
            msg.extend_from_slice(&[0; 3]);
            msg.extend_from_slice(&chain.level_db.to_le_bytes());
            if c.audio_on {
                for a in &audio {
                    msg.extend_from_slice(&((a.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
                }
            }
            let _ = c.tx.try_send(Message::Binary(msg.into()));
        }
    }
    tracing::info!("DSP-Thread beendet (Quelle geschlossen)");
}
