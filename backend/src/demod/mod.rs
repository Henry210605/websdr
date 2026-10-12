//! Empfangskette eines VFO: Mischen, Dezimieren, Kanalfilter, Demodulation, Audio 48 kHz.

use num_complex::Complex32;
use serde::{Deserialize, Serialize};

use crate::dsp::{lowpass, Agc, DcBlock, Deemphasis, FirDecim, FirDecimReal, Nco, Resampler};

pub const AUDIO_RATE: f64 = 48_000.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Am,
    Nfm,
    #[default]
    Wfm,
    Usb,
    Lsb,
    Cw,
    Dsb,
}

impl Mode {
    pub fn default_bandwidth(self) -> u32 {
        match self {
            Mode::Am => 10_000,
            Mode::Nfm => 12_500,
            Mode::Wfm => 200_000,
            Mode::Usb | Mode::Lsb => 2_800,
            Mode::Cw => 500,
            Mode::Dsb => 6_000,
        }
    }

    pub fn bandwidth_limits(self) -> (u32, u32) {
        match self {
            Mode::Wfm => (50_000, 250_000),
            Mode::Cw => (50, 3_000),
            Mode::Usb | Mode::Lsb => (500, 6_000),
            _ => (1_000, 40_000),
        }
    }
}

/// Tonhöhe des CW-Tons (BFO-Versatz).
const CW_PITCH_HZ: f64 = 700.0;
/// Untere Kante des SSB-Durchlassbereichs.
const SSB_LOW_HZ: f64 = 100.0;

struct Config {
    mode: Mode,
    bandwidth: u32,
    in_rate: u32,
}

pub struct Chain {
    cfg: Config,
    nco: Nco,
    dec: FirDecim,
    chan: Option<FirDecim>,
    /// Verschiebung für SSB/CW innerhalb des ZF-Signals (Weaver-Prinzip).
    shift: Option<(Nco, Nco)>,
    prev: Complex32,
    fm_gain: f32,
    deemph: Option<Deemphasis>,
    audio_lpf: Option<FirDecimReal>,
    resampler: Resampler,
    agc: Option<Agc>,
    dc: Option<DcBlock>,
    /// Zuletzt gemessene Kanalleistung in dBFS (für S-Meter und Rauschsperre).
    pub level_db: f32,
    // Arbeitspuffer
    mixed: Vec<Complex32>,
    if_buf: Vec<Complex32>,
    chan_buf: Vec<Complex32>,
    audio_if: Vec<f32>,
    audio_tmp: Vec<f32>,
}

impl Chain {
    pub fn new(mode: Mode, bandwidth: u32, in_rate: u32) -> Self {
        let fs = in_rate as f64;
        let bw = bandwidth as f64;
        let max_taps = 1023;

        // Stufe 1: auf eine Zwischenrate dezimieren, die zum Modus passt.
        let target_if = if mode == Mode::Wfm { 240_000.0 } else { (bw * 1.6).max(48_000.0) };
        let decim = ((fs / target_if).floor() as usize).max(1);
        let if_rate = fs / decim as f64;
        // Durchlassbereich der ersten Stufe: bei SSB liegt das Signal einseitig.
        let pass = match mode {
            Mode::Usb | Mode::Lsb => bw + SSB_LOW_HZ,
            _ => bw / 2.0,
        }
        .min(if_rate * 0.45);
        // Erste Stufe nur als Anti-Aliasing-Filter: Alles oberhalb von `stop` würde in den
        // Durchlassbereich gespiegelt. Die breite Übergangszone hält die Filterlänge klein.
        let stop = (if_rate - pass).max(pass * 1.1);
        let dec = FirDecim::new(lowpass((pass + stop) / 2.0 / fs, (stop - pass).max(1_000.0) / fs, max_taps), decim);

        // Kanalfilter auf der Zwischenrate (schmal, aber bei niedriger Rate günstig).
        let chan = {
            let half = match mode {
                Mode::Usb | Mode::Lsb => (bw - SSB_LOW_HZ) / 2.0,
                _ => bw / 2.0,
            }
            .min(if_rate * 0.45);
            let trans = (half * 0.3).max(150.0).min(if_rate / 2.0 - half).max(if_rate * 0.02);
            Some(FirDecim::new(lowpass(half / if_rate, trans / if_rate, 511), 1))
        };
        // (Verschiebung vor dem Kanalfilter, Verschiebung danach): SSB wird um die Mitte des
        // Seitenbands gefiltert und zurückgeschoben; CW wird um den Träger gefiltert und
        // anschließend um die Tonhöhe verschoben.
        let shift = match mode {
            Mode::Usb => Some(((bw + SSB_LOW_HZ) / 2.0, (bw + SSB_LOW_HZ) / 2.0)),
            Mode::Lsb => Some((-(bw + SSB_LOW_HZ) / 2.0, -(bw + SSB_LOW_HZ) / 2.0)),
            Mode::Cw => Some((0.0, CW_PITCH_HZ)),
            _ => None,
        }
        .map(|(down, up)| (Nco::new(-down, if_rate), Nco::new(up, if_rate)));

        let (deemph, audio_lpf, audio_rate) = if mode == Mode::Wfm {
            let d = ((if_rate / AUDIO_RATE).floor() as usize).max(1);
            let lpf = FirDecimReal::new(lowpass(15_000.0 / if_rate, 4_000.0 / if_rate, 255), d);
            (Some(Deemphasis::new(if_rate as f32, 50e-6)), Some(lpf), if_rate / d as f64)
        } else {
            (None, None, if_rate)
        };
        let fm_dev = match mode {
            Mode::Wfm => 75_000.0,
            _ => (bw / 2.5).max(2_500.0),
        };
        let agc = matches!(mode, Mode::Am | Mode::Usb | Mode::Lsb | Mode::Cw | Mode::Dsb)
            .then(|| Agc::new(AUDIO_RATE as f32));
        let dc = (mode == Mode::Am).then(|| DcBlock::new(if_rate as f32, 30.0));

        Self {
            cfg: Config { mode, bandwidth, in_rate },
            nco: Nco::new(0.0, fs),
            dec,
            chan,
            shift,
            prev: Complex32::new(1.0, 0.0),
            fm_gain: (if_rate / (std::f64::consts::TAU * fm_dev)) as f32,
            deemph,
            audio_lpf,
            resampler: Resampler::new(audio_rate, AUDIO_RATE),
            agc,
            dc,
            level_db: -150.0,
            mixed: Vec::new(),
            if_buf: Vec::new(),
            chan_buf: Vec::new(),
            audio_if: Vec::new(),
            audio_tmp: Vec::new(),
        }
    }

    pub fn matches(&self, mode: Mode, bandwidth: u32, in_rate: u32) -> bool {
        self.cfg.mode == mode && self.cfg.bandwidth == bandwidth && self.cfg.in_rate == in_rate
    }

    /// Verarbeitet einen IQ-Block. `offset_hz` = VFO − Mitte. Hängt Audio (48 kHz) an `out` an.
    pub fn process(&mut self, iq: &[Complex32], offset_hz: f64, squelch_db: Option<f32>, out: &mut Vec<f32>) {
        let in_band = offset_hz.abs() < self.cfg.in_rate as f64 / 2.0;
        self.nco.set_freq(-offset_hz);
        self.mixed.clear();
        self.mixed.extend_from_slice(iq);
        self.nco.mix(&mut self.mixed);

        self.if_buf.clear();
        self.dec.process(&self.mixed, &mut self.if_buf);

        // Kanalfilter (bei SSB/CW in das verschobene Signal)
        let chan: &[Complex32] = if let Some(f) = &mut self.chan {
            if let Some((down, _)) = &mut self.shift {
                down.mix(&mut self.if_buf);
            }
            self.chan_buf.clear();
            f.process(&self.if_buf, &mut self.chan_buf);
            &self.chan_buf
        } else {
            &self.if_buf
        };

        let p = chan.iter().map(|s| s.norm_sqr()).sum::<f32>() / chan.len().max(1) as f32;
        self.level_db = if in_band { 10.0 * (p + 1e-20).log10() } else { -150.0 };

        self.audio_if.clear();
        match self.cfg.mode {
            Mode::Wfm | Mode::Nfm => {
                let mut prev = self.prev;
                for s in chan {
                    let d = s * prev.conj();
                    self.audio_if.push(d.im.atan2(d.re) * self.fm_gain);
                    prev = *s;
                }
                self.prev = prev;
            }
            Mode::Am => {
                self.audio_if.extend(chan.iter().map(|s| s.norm()));
                if let Some(dc) = &mut self.dc {
                    dc.process(&mut self.audio_if);
                }
            }
            Mode::Usb | Mode::Lsb | Mode::Cw => {
                let (_, up) = self.shift.as_mut().unwrap();
                for s in chan {
                    self.audio_if.push((s * up.next()).re);
                }
                up.renormalize();
            }
            Mode::Dsb => self.audio_if.extend(chan.iter().map(|s| s.re)),
        }
        if let Some(d) = &mut self.deemph {
            d.process(&mut self.audio_if);
        }
        let src: &[f32] = if let Some(lpf) = &mut self.audio_lpf {
            self.audio_tmp.clear();
            lpf.process(&self.audio_if, &mut self.audio_tmp);
            &self.audio_tmp
        } else {
            &self.audio_if
        };
        let start = out.len();
        self.resampler.process(src, out);
        let audio = &mut out[start..];
        if let Some(agc) = &mut self.agc {
            agc.process(audio);
        }
        let muted = !in_band || squelch_db.is_some_and(|sq| self.level_db < sq);
        if muted {
            audio.iter_mut().for_each(|s| *s = 0.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::TAU;

    /// Erzeugt ein FM-Signal mit Ton `tone_hz` bei Versatz `off` und prüft die Tonfrequenz im Audio.
    fn dominant_freq(audio: &[f32], rate: f64) -> f64 {
        // Nulldurchgänge zählen
        let skip = audio.len() / 4;
        let a = &audio[skip..];
        let crossings = a.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        crossings as f64 / (a.len() as f64 / rate)
    }

    fn run(mode: Mode, gen: impl Fn(f64) -> Complex32, rate: u32, offset: f64) -> Vec<f32> {
        let mut c = Chain::new(mode, mode.default_bandwidth(), rate);
        let n = rate as usize / 2;
        let iq: Vec<Complex32> = (0..n).map(|i| gen(i as f64 / rate as f64)).collect();
        let mut out = Vec::new();
        for b in iq.chunks(32768) {
            c.process(b, offset, None, &mut out);
        }
        out
    }

    #[test]
    fn audio_rate_is_48k() {
        for rate in [2_400_000, 2_048_000, 1_024_000, 250_000] {
            for mode in [Mode::Wfm, Mode::Nfm, Mode::Am, Mode::Usb] {
                let out = run(mode, |_| Complex32::new(0.0, 0.0), rate, 0.0);
                let expected = 24_000.0;
                assert!((out.len() as f64 - expected).abs() < 300.0, "{mode:?} {rate}: {}", out.len());
            }
        }
    }

    #[test]
    fn nfm_tone() {
        let rate = 2_400_000.0;
        let off = 300_000.0;
        let gen = |t: f64| {
            let ph = TAU * off * t + 2_500.0 / 1_000.0 * (TAU * 1_000.0 * t).sin();
            Complex32::new(ph.cos() as f32, ph.sin() as f32)
        };
        let out = run(Mode::Nfm, gen, rate as u32, off);
        let f = dominant_freq(&out, AUDIO_RATE);
        assert!((f - 1_000.0).abs() < 20.0, "{f}");
    }

    #[test]
    fn wfm_tone() {
        let rate = 2_400_000.0;
        let off = -400_000.0;
        let gen = |t: f64| {
            let ph = TAU * off * t + 50_000.0 / 1_000.0 * (TAU * 1_000.0 * t).sin();
            Complex32::new(ph.cos() as f32, ph.sin() as f32)
        };
        let out = run(Mode::Wfm, gen, rate as u32, off);
        let f = dominant_freq(&out, AUDIO_RATE);
        assert!((f - 1_000.0).abs() < 20.0, "{f}");
    }

    #[test]
    fn am_tone() {
        let rate = 2_048_000.0;
        let off = 100_000.0;
        let gen = |t: f64| {
            let a = 0.1 * (1.0 + 0.5 * (TAU * 600.0 * t).sin());
            let ph = TAU * off * t;
            Complex32::new((a * ph.cos()) as f32, (a * ph.sin()) as f32)
        };
        let out = run(Mode::Am, gen, rate as u32, off);
        let f = dominant_freq(&out, AUDIO_RATE);
        assert!((f - 600.0).abs() < 15.0, "{f}");
    }

    #[test]
    fn usb_and_lsb_select_sideband() {
        let rate = 2_400_000.0;
        let off = 50_000.0;
        // Ton 1 kHz oberhalb des Trägers
        let gen = |t: f64| {
            let ph = TAU * (off + 1_000.0) * t;
            Complex32::new((0.01 * ph.cos()) as f32, (0.01 * ph.sin()) as f32)
        };
        let usb = run(Mode::Usb, gen, rate as u32, off);
        let f = dominant_freq(&usb, AUDIO_RATE);
        assert!((f - 1_000.0).abs() < 20.0, "USB {f}");

        let mut c = Chain::new(Mode::Lsb, 2_800, rate as u32);
        let iq: Vec<Complex32> = (0..rate as usize / 2).map(|i| gen(i as f64 / rate)).collect();
        let mut out = Vec::new();
        for b in iq.chunks(32768) {
            c.process(b, off, None, &mut out);
        }
        // LSB darf das obere Seitenband kaum durchlassen: Pegel deutlich unter USB-Pegel
        let usb_c = {
            let mut c = Chain::new(Mode::Usb, 2_800, rate as u32);
            let mut o = Vec::new();
            for b in iq.chunks(32768) {
                c.process(b, off, None, &mut o);
            }
            c.level_db
        };
        assert!(c.level_db < usb_c - 30.0, "LSB {} USB {}", c.level_db, usb_c);
    }
}

#[cfg(test)]
mod bench {
    use super::*;
    use crate::dsp::spectrum::Spectrum;
    use std::time::Instant;

    /// Rechenzeit für eine Sekunde Signal bei 2,4 MS/s: `cargo test --release bench -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn cpu_per_second_of_signal() {
        let rate = 2_400_000u32;
        let iq: Vec<Complex32> = (0..rate as usize)
            .map(|i| Complex32::new(((i * 7919) % 255) as f32 / 255.0 - 0.5, ((i * 104729) % 255) as f32 / 255.0 - 0.5))
            .collect();
        let mut s = Spectrum::new(8192, rate, 25);
        let t = Instant::now();
        for b in iq.chunks(32768) {
            s.push(b, 100_000_000);
        }
        println!("Spektrum 8192: {:.1} ms/s", t.elapsed().as_secs_f64() * 1e3);
        for mode in [Mode::Wfm, Mode::Nfm, Mode::Am, Mode::Usb, Mode::Cw] {
            let mut c = Chain::new(mode, mode.default_bandwidth(), rate);
            let mut out = Vec::new();
            let t = Instant::now();
            for b in iq.chunks(32768) {
                out.clear();
                c.process(b, 300_000.0, None, &mut out);
            }
            println!("{mode:?}: {:.1} ms/s", t.elapsed().as_secs_f64() * 1e3);
        }
    }
}
