//! Grundbausteine der Signalverarbeitung.

pub mod spectrum;

use std::f64::consts::TAU;

use num_complex::{Complex, Complex32};

/// Numerisch gesteuerter Oszillator: mischt ein komplexes Signal um `freq` Hz.
pub struct Nco {
    phasor: Complex<f64>,
    step: Complex<f64>,
    freq: f64,
    rate: f64,
}

impl Nco {
    pub fn new(freq: f64, rate: f64) -> Self {
        let mut n = Self {
            phasor: Complex::new(1.0, 0.0),
            step: Complex::new(1.0, 0.0),
            freq: 0.0,
            rate,
        };
        n.set_freq(freq);
        n
    }

    pub fn set_freq(&mut self, freq: f64) {
        if freq != self.freq {
            self.freq = freq;
            self.step = Complex::from_polar(1.0, TAU * freq / self.rate);
        }
    }

    #[inline]
    pub fn next(&mut self) -> Complex32 {
        let p = self.phasor;
        self.phasor *= self.step;
        Complex32::new(p.re as f32, p.im as f32)
    }

    /// Multipliziert jedes Sample mit dem Oszillator.
    pub fn mix(&mut self, buf: &mut [Complex32]) {
        for s in buf.iter_mut() {
            *s *= self.next();
        }
        self.renormalize();
    }

    pub fn renormalize(&mut self) {
        self.phasor /= self.phasor.norm();
    }
}

/// Entwurf eines Tiefpasses (gefenstertes sinc, Blackman-Fenster).
/// `cutoff` und `transition` relativ zur Abtastrate (0..0,5).
pub fn lowpass(cutoff: f64, transition: f64, max_taps: usize) -> Vec<f32> {
    let mut n = (5.5 / transition.max(1e-4)).ceil() as usize;
    n = n.clamp(15, max_taps.max(15));
    if n % 2 == 0 {
        n += 1;
    }
    let m = (n - 1) as f64;
    let mut taps: Vec<f64> = (0..n)
        .map(|i| {
            let x = i as f64 - m / 2.0;
            let sinc = if x == 0.0 {
                2.0 * cutoff
            } else {
                (TAU * cutoff * x).sin() / (std::f64::consts::PI * x)
            };
            let w = 0.42 - 0.5 * (TAU * i as f64 / m).cos() + 0.08 * (2.0 * TAU * i as f64 / m).cos();
            sinc * w
        })
        .collect();
    let sum: f64 = taps.iter().sum();
    taps.iter_mut().for_each(|t| *t /= sum);
    taps.into_iter().map(|t| t as f32).collect()
}

/// Skalarprodukt mit 8 Teilsummen, damit der Compiler SIMD-Befehle nutzen kann.
#[inline]
pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    let mut acc = [0.0f32; 8];
    let ca = a.chunks_exact(8);
    let cb = b.chunks_exact(8);
    let (ra, rb) = (ca.remainder(), cb.remainder());
    for (x, y) in ca.zip(cb) {
        for i in 0..8 {
            acc[i] += x[i] * y[i];
        }
    }
    let mut sum: f32 = acc.iter().sum();
    for (x, y) in ra.iter().zip(rb) {
        sum += x * y;
    }
    sum
}

/// FIR-Filter mit reellen Koeffizienten und Dezimation für komplexe Daten.
/// Es werden nur die Ausgangswerte berechnet, die nach der Dezimation übrig bleiben.
/// Real- und Imaginärteil liegen getrennt, damit die Skalarprodukte vektorisiert werden.
pub struct FirDecim {
    taps: Vec<f32>,
    decim: usize,
    re: Vec<f32>,
    im: Vec<f32>,
    /// Index des nächsten zu berechnenden Ausgangs relativ zum Anfang des Verlaufs.
    pos: usize,
}

impl FirDecim {
    pub fn new(mut taps: Vec<f32>, decim: usize) -> Self {
        let n = taps.len();
        taps.reverse();
        Self { taps, decim: decim.max(1), re: vec![0.0; n - 1], im: vec![0.0; n - 1], pos: 0 }
    }

    pub fn process(&mut self, input: &[Complex32], out: &mut Vec<Complex32>) {
        let n = self.taps.len();
        self.re.extend(input.iter().map(|s| s.re));
        self.im.extend(input.iter().map(|s| s.im));
        while self.pos + n <= self.re.len() {
            let r = dot(&self.taps, &self.re[self.pos..self.pos + n]);
            let i = dot(&self.taps, &self.im[self.pos..self.pos + n]);
            out.push(Complex32::new(r, i));
            self.pos += self.decim;
        }
        // Verbrauchte Samples entfernen, (n-1) Samples Vorgeschichte behalten.
        let consumed = self.pos.min(self.re.len());
        self.re.drain(..consumed);
        self.im.drain(..consumed);
        self.pos -= consumed;
    }
}

/// Reelles FIR-Filter mit Dezimation (für Audio).
pub struct FirDecimReal {
    taps: Vec<f32>,
    decim: usize,
    hist: Vec<f32>,
    pos: usize,
}

impl FirDecimReal {
    pub fn new(mut taps: Vec<f32>, decim: usize) -> Self {
        let n = taps.len();
        taps.reverse();
        Self { taps, decim: decim.max(1), hist: vec![0.0; n - 1], pos: 0 }
    }

    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        let n = self.taps.len();
        self.hist.extend_from_slice(input);
        while self.pos + n <= self.hist.len() {
            out.push(dot(&self.taps, &self.hist[self.pos..self.pos + n]));
            self.pos += self.decim;
        }
        let consumed = self.pos.min(self.hist.len());
        self.hist.drain(..consumed);
        self.pos -= consumed;
    }
}

/// Linearer Resampler für bereits tiefpassgefiltertes Audio.
pub struct Resampler {
    step: f64,
    pos: f64,
    last: f32,
}

impl Resampler {
    pub fn new(in_rate: f64, out_rate: f64) -> Self {
        Self { step: in_rate / out_rate, pos: 0.0, last: 0.0 }
    }

    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        // pos ist relativ zu `last` (Index -1) + 1, d. h. pos=0 zeigt auf `last`.
        let get = |i: usize| if i == 0 { self.last } else { input[i - 1] };
        let len = input.len() as f64;
        while self.pos < len {
            let i = self.pos.floor();
            let frac = (self.pos - i) as f32;
            let a = get(i as usize);
            let b = get(i as usize + 1);
            out.push(a + (b - a) * frac);
            self.pos += self.step;
        }
        self.pos -= len;
        if let Some(l) = input.last() {
            self.last = *l;
        }
    }
}

/// Einfache Audio-AGC mit schneller Attack- und langsamer Release-Zeit.
pub struct Agc {
    env: f32,
    attack: f32,
    release: f32,
    target: f32,
    max_gain: f32,
}

impl Agc {
    pub fn new(rate: f32) -> Self {
        Self {
            env: 1e-3,
            attack: 1.0 - (-1.0 / (0.002 * rate)).exp(),
            release: 1.0 - (-1.0 / (0.5 * rate)).exp(),
            target: 0.4,
            max_gain: 10_000.0,
        }
    }

    pub fn process(&mut self, buf: &mut [f32]) {
        for s in buf.iter_mut() {
            let a = s.abs();
            let k = if a > self.env { self.attack } else { self.release };
            self.env += k * (a - self.env);
            let g = (self.target / self.env.max(1e-9)).min(self.max_gain);
            *s = (*s * g).clamp(-1.0, 1.0);
        }
    }
}

/// Gleichanteil entfernen (Hochpass erster Ordnung).
pub struct DcBlock {
    x1: f32,
    y1: f32,
    r: f32,
}

impl DcBlock {
    pub fn new(rate: f32, cutoff_hz: f32) -> Self {
        Self { x1: 0.0, y1: 0.0, r: (-std::f32::consts::TAU * cutoff_hz / rate).exp() }
    }

    pub fn process(&mut self, buf: &mut [f32]) {
        for s in buf.iter_mut() {
            let y = *s - self.x1 + self.r * self.y1;
            self.x1 = *s;
            self.y1 = y;
            *s = y;
        }
    }
}

/// Deemphase für FM (Tiefpass erster Ordnung mit Zeitkonstante tau).
pub struct Deemphasis {
    alpha: f32,
    y: f32,
}

impl Deemphasis {
    pub fn new(rate: f32, tau_s: f32) -> Self {
        Self { alpha: 1.0 - (-1.0 / (rate * tau_s)).exp(), y: 0.0 }
    }

    pub fn process(&mut self, buf: &mut [f32]) {
        for s in buf.iter_mut() {
            self.y += self.alpha * (*s - self.y);
            *s = self.y;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(freq: f64, rate: f64, n: usize) -> Vec<Complex32> {
        let mut nco = Nco::new(freq, rate);
        (0..n).map(|_| nco.next()).collect()
    }

    fn power(x: &[Complex32]) -> f32 {
        x.iter().map(|s| s.norm_sqr()).sum::<f32>() / x.len() as f32
    }

    #[test]
    fn lowpass_passes_and_blocks() {
        let rate = 240_000.0;
        let taps = lowpass(10_000.0 / rate, 5_000.0 / rate, 1023);
        let mut f = FirDecim::new(taps.clone(), 1);
        let mut out = Vec::new();
        f.process(&tone(3_000.0, rate, 20_000), &mut out);
        assert!((power(&out[taps.len()..]) - 1.0).abs() < 0.05);
        let mut f = FirDecim::new(taps.clone(), 1);
        out.clear();
        f.process(&tone(30_000.0, rate, 20_000), &mut out);
        assert!(power(&out[taps.len()..]) < 1e-4);
    }

    #[test]
    fn decimator_block_boundaries_are_seamless() {
        let rate = 2_400_000.0;
        let taps = lowpass(20_000.0 / rate, 40_000.0 / rate, 1023);
        let x = tone(5_000.0, rate, 100_000);
        let mut a = FirDecim::new(taps.clone(), 10);
        let mut b = FirDecim::new(taps, 10);
        let mut out_a = Vec::new();
        let mut out_b = Vec::new();
        a.process(&x, &mut out_a);
        for chunk in x.chunks(3333) {
            b.process(chunk, &mut out_b);
        }
        assert_eq!(out_a.len(), out_b.len());
        for (p, q) in out_a.iter().zip(&out_b) {
            assert!((p - q).norm() < 1e-5);
        }
    }

    #[test]
    fn resampler_ratio() {
        let mut r = Resampler::new(51_200.0, 48_000.0);
        let mut out = Vec::new();
        for _ in 0..100 {
            r.process(&vec![0.5; 512], &mut out);
        }
        let expected = 48_000.0;
        assert!((out.len() as f64 - expected).abs() < 2.0, "{}", out.len());
    }
}
