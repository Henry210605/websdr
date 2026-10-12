//! Simulierte Quelle für Entwicklung und Tests ohne Stick.
//!
//! Erzeugt in Echtzeit ein Rauschband mit einigen Sendern um 100 MHz:
//! WFM 100,0 MHz, NFM 100,3 MHz (getastet), AM 99,5 MHz, USB 99,3 MHz, CW 99,2 MHz.

use std::f64::consts::TAU;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use num_complex::Complex32;

use super::{push_block, set_status, IqBlock, SharedStatus, SourceCmd, BLOCK_SAMPLES, R82XX_GAINS};
use crate::state::DeviceSettings;

#[derive(Clone, Copy)]
enum Kind {
    Wfm,
    Nfm,
    Am,
    Usb,
    Cw,
}

struct Station {
    freq: f64,
    kind: Kind,
    amp: f64,
    carrier_phase: f64,
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64 - 0.5
    }
}

pub fn run(
    mut settings: DeviceSettings,
    cmds: Receiver<SourceCmd>,
    out: Sender<IqBlock>,
    status: SharedStatus,
) {
    {
        let mut s = status.lock();
        s.device_name = "Simulator".into();
        s.gains = R82XX_GAINS.iter().map(|g| *g as f32 / 10.0).collect();
    }
    set_status(&status, true, "Simulator aktiv (kein echter Stick)");

    let mut stations = vec![
        Station { freq: 100.0e6, kind: Kind::Wfm, amp: 0.25, carrier_phase: 0.0 },
        Station { freq: 100.3e6, kind: Kind::Nfm, amp: 0.05, carrier_phase: 0.0 },
        Station { freq: 99.5e6, kind: Kind::Am, amp: 0.08, carrier_phase: 0.0 },
        Station { freq: 99.3e6, kind: Kind::Usb, amp: 0.03, carrier_phase: 0.0 },
        Station { freq: 99.2e6, kind: Kind::Cw, amp: 0.02, carrier_phase: 0.0 },
    ];
    let mut rng = Rng(0x9E3779B97F4A7C15);
    let mut t_samples: u64 = 0; // fortlaufende Zeit in Samples (für Modulation)
    let mut fm_phase = [0.0f64; 2];
    let mut start = Instant::now();
    let mut sent_samples: u64 = 0;

    loop {
        while let Ok(cmd) = cmds.try_recv() {
            match cmd {
                SourceCmd::CenterFreq(hz) => settings.center_hz = hz,
                SourceCmd::SampleRate(r) => {
                    settings.sample_rate = r;
                    start = Instant::now();
                    sent_samples = 0;
                }
                SourceCmd::Gain(g) => settings.gain_db = g,
                SourceCmd::Ppm(p) => settings.ppm = p,
                SourceCmd::BiasTee(b) => settings.bias_tee = b,
            }
        }
        let fs = settings.sample_rate as f64;
        let center = settings.center_hz as f64;
        // Verstärkung grob nachbilden: mehr Gain hebt Signal und Rauschen an.
        let gain = 10f64.powf((settings.gain_db.unwrap_or(30.0) as f64 - 30.0) / 20.0);
        let noise_amp = 0.02 * gain;

        let mut buf = vec![Complex32::new(0.0, 0.0); BLOCK_SAMPLES];
        for (i, s) in buf.iter_mut().enumerate() {
            let t = (t_samples + i as u64) as f64 / fs;
            let mut re = noise_amp * (rng.next() + rng.next() + rng.next());
            let mut im = noise_amp * (rng.next() + rng.next() + rng.next());
            for (k, st) in stations.iter_mut().enumerate() {
                let off = st.freq - center;
                if off.abs() > fs / 2.0 {
                    continue;
                }
                st.carrier_phase = (st.carrier_phase + TAU * off / fs) % TAU;
                let a = st.amp * gain;
                let (amp, phase) = match st.kind {
                    Kind::Wfm => {
                        // Melodie aus wechselnden Tönen, Hub 50 kHz.
                        let note = [440.0, 554.4, 659.3, 880.0][(t * 2.0) as usize % 4];
                        let m = 0.6 * (TAU * note * t).sin() + 0.3 * (TAU * 220.0 * t).sin();
                        fm_phase[0] += TAU * 50_000.0 * m / fs;
                        (a, fm_phase[0])
                    }
                    Kind::Nfm => {
                        let keyed = (t as u64 / 2) % 2 == 0;
                        if !keyed {
                            continue;
                        }
                        fm_phase[1] += TAU * 2_500.0 * (TAU * 1000.0 * t).sin() / fs;
                        (a, fm_phase[1])
                    }
                    Kind::Am => (a * (1.0 + 0.6 * (TAU * 600.0 * t).sin()), 0.0),
                    Kind::Usb => {
                        // Zwei-Ton-Signal: 700 Hz und 1500 Hz oberhalb der Trägerfrequenz.
                        let p1 = TAU * 700.0 * t;
                        let p2 = TAU * 1500.0 * t;
                        let (re2, im2) = (p1.cos() + p2.cos(), p1.sin() + p2.sin());
                        let (c, sn) = (st.carrier_phase.cos(), st.carrier_phase.sin());
                        re += a * 0.5 * (re2 * c - im2 * sn);
                        im += a * 0.5 * (re2 * sn + im2 * c);
                        let _ = k;
                        continue;
                    }
                    Kind::Cw => {
                        // "CQ" in Morse, ca. 15 WpM, als Ein/Aus-Tastung.
                        const PATTERN: &[u8] = b"1110101110100011101110101110000000";
                        let dot = 0.08;
                        let idx = (t / dot) as usize % PATTERN.len();
                        if PATTERN[idx] != b'1' {
                            continue;
                        }
                        (a, 0.0)
                    }
                };
                let p = st.carrier_phase + phase;
                re += amp * p.cos();
                im += amp * p.sin();
            }
            *s = Complex32::new(re as f32, im as f32);
        }
        fm_phase[0] %= TAU;
        fm_phase[1] %= TAU;
        t_samples += BLOCK_SAMPLES as u64;
        sent_samples += BLOCK_SAMPLES as u64;

        // Echtzeit einhalten.
        let due = start + Duration::from_secs_f64(sent_samples as f64 / fs);
        let now = Instant::now();
        if due > now {
            std::thread::sleep(due - now);
        } else if now - due > Duration::from_millis(500) {
            // Zu langsam: nicht aufholen, sondern Zeitbasis neu setzen.
            start = now;
            sent_samples = 0;
        }

        let block = IqBlock {
            center_hz: settings.center_hz,
            sample_rate: settings.sample_rate,
            samples: buf,
        };
        if !push_block(&out, block, &status) {
            return;
        }
    }
}
