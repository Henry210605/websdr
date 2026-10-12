//! Leistungsspektrum für Spektrumanzeige und Wasserfall.

use std::sync::Arc;

use num_complex::Complex32;
use rustfft::{Fft, FftPlanner};

/// dB-Bereich, der in die 8-Bit-Werte für den Browser abgebildet wird.
pub const DB_MIN: f32 = -150.0;
pub const DB_MAX: f32 = 0.0;

pub struct Spectrum {
    size: usize,
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    norm: f32,
    buf: Vec<Complex32>,
    work: Vec<Complex32>,
    scratch: Vec<Complex32>,
    acc: Vec<f32>,
    acc_count: usize,
    since_frame: usize,
    rate: u32,
    fps: u32,
}

/// Ein fertiges, über ein Bild-Intervall gemitteltes Spektrum in dBFS (fftshift: tiefe Frequenzen links).
pub struct Frame {
    pub center_hz: u64,
    pub sample_rate: u32,
    pub db: Vec<f32>,
}

impl Spectrum {
    pub fn new(size: usize, rate: u32, fps: u32) -> Self {
        let fft = FftPlanner::new().plan_fft_forward(size);
        let window: Vec<f32> = (0..size)
            .map(|i| {
                // Blackman-Harris (4 Terme)
                let x = std::f64::consts::TAU * i as f64 / (size - 1) as f64;
                (0.35875 - 0.48829 * x.cos() + 0.14128 * (2.0 * x).cos() - 0.01168 * (3.0 * x).cos())
                    as f32
            })
            .collect();
        let sum: f32 = window.iter().sum();
        let scratch_len = fft.get_inplace_scratch_len();
        Self {
            size,
            fft,
            window,
            norm: 1.0 / (sum * sum),
            buf: Vec::with_capacity(size * 2),
            work: vec![Complex32::new(0.0, 0.0); size],
            scratch: vec![Complex32::new(0.0, 0.0); scratch_len],
            acc: vec![0.0; size],
            acc_count: 0,
            since_frame: 0,
            rate,
            fps,
        }
    }

    pub fn size(&self) -> usize {
        self.size
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// Verarbeitet einen Block; liefert ein Frame, sobald ein Bild-Intervall voll ist.
    pub fn push(&mut self, samples: &[Complex32], center_hz: u64) -> Option<Frame> {
        let frame_len = (self.rate / self.fps).max(1) as usize;
        // Bei großen FFTs und kleinen Raten überlappen, damit die Bildrate erreicht wird.
        let hop = self.size.min(frame_len);
        self.buf.extend_from_slice(samples);
        let mut frame = None;
        while self.buf.len() >= self.size {
            for ((w, s), win) in self.work.iter_mut().zip(&self.buf[..self.size]).zip(&self.window) {
                *w = s * *win;
            }
            self.fft.process_with_scratch(&mut self.work, &mut self.scratch);
            for (a, w) in self.acc.iter_mut().zip(&self.work) {
                *a += w.norm_sqr();
            }
            self.acc_count += 1;
            self.buf.drain(..hop);
            self.since_frame += hop;
            if self.since_frame >= frame_len {
                self.since_frame = 0;
                frame = Some(self.take_frame(center_hz));
            }
        }
        frame
    }

    fn take_frame(&mut self, center_hz: u64) -> Frame {
        let n = self.size;
        let scale = self.norm / self.acc_count.max(1) as f32;
        let mut db = vec![0.0f32; n];
        for (i, a) in self.acc.iter().enumerate() {
            // fftshift: Bin 0 (DC) in die Mitte
            let j = (i + n / 2) % n;
            db[j] = 10.0 * (a * scale + 1e-20).log10();
        }
        self.acc.iter_mut().for_each(|a| *a = 0.0);
        self.acc_count = 0;
        Frame { center_hz, sample_rate: self.rate, db }
    }
}

/// Rechnet ein Frame auf die Pixelbreite eines Browsers herunter (Max-Hold je Pixel,
/// damit schmale Träger sichtbar bleiben) und quantisiert auf 8 Bit.
pub fn render(db: &[f32], zoom: (f64, f64), width: usize, out: &mut Vec<u8>) {
    let n = db.len() as f64;
    let (z0, z1) = zoom;
    let start = z0 * n;
    let span = (z1 - z0) * n;
    let per_px = span / width as f64;
    let scale = 255.0 / (DB_MAX - DB_MIN);
    out.clear();
    for x in 0..width {
        let a = start + x as f64 * per_px;
        let b = a + per_px;
        let i0 = (a.floor() as usize).min(db.len() - 1);
        let i1 = (b.ceil() as usize).clamp(i0 + 1, db.len());
        let v = db[i0..i1].iter().cloned().fold(f32::MIN, f32::max);
        out.push(((v - DB_MIN) * scale).clamp(0.0, 255.0) as u8);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::Nco;

    #[test]
    fn tone_appears_at_right_bin_with_0dbfs() {
        let rate = 2_400_000;
        let mut s = Spectrum::new(4096, rate, 25);
        let mut nco = Nco::new(300_000.0, rate as f64);
        let x: Vec<_> = (0..rate as usize / 10).map(|_| nco.next()).collect();
        let mut frame = None;
        for c in x.chunks(32768) {
            if let Some(f) = s.push(c, 100_000_000) {
                frame = Some(f);
            }
        }
        let f = frame.expect("kein Frame");
        let (imax, vmax) = f
            .db
            .iter()
            .enumerate()
            .fold((0, f32::MIN), |m, (i, v)| if *v > m.1 { (i, *v) } else { m });
        let expected_bin = 2048 + (300_000.0 / rate as f64 * 4096.0).round() as usize;
        assert!((imax as i64 - expected_bin as i64).abs() <= 1, "{imax} vs {expected_bin}");
        assert!(vmax.abs() < 1.0, "Pegel {vmax}");
    }

    #[test]
    fn render_keeps_peaks() {
        let mut db = vec![-120.0f32; 8192];
        db[5000] = -20.0;
        let mut out = Vec::new();
        render(&db, (0.0, 1.0), 800, &mut out);
        assert_eq!(out.len(), 800);
        assert!(out.iter().any(|v| *v > 200));
    }
}
