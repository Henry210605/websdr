/** Frequenz in Hz als "145.500.000" (Tausenderpunkte). */
export function formatHz(hz: number): string {
  return Math.round(hz).toString().replace(/\B(?=(\d{3})+(?!\d))/g, '.');
}

/** Kurzform für Achsen und Listen: "145,500 MHz", "7,100 MHz", "999 kHz". */
export function formatShort(hz: number, decimals = 3): string {
  if (Math.abs(hz) >= 1e6) return (hz / 1e6).toFixed(decimals).replace('.', ',') + ' MHz';
  if (Math.abs(hz) >= 1e3) return (hz / 1e3).toFixed(decimals > 1 ? 1 : 0).replace('.', ',') + ' kHz';
  return Math.round(hz) + ' Hz';
}

export function formatBw(hz: number): string {
  return hz >= 1000 ? (hz / 1000).toLocaleString('de-DE', { maximumFractionDigits: 2 }) + ' kHz' : hz + ' Hz';
}

/**
 * Liest eine Frequenzeingabe: "145.5M", "145,5 MHz", "7100k", "100000000".
 * Ohne Einheit: Werte unter 2000 gelten als MHz, sonst als Hz.
 */
export function parseFreq(input: string): number | null {
  const s = input.trim().toLowerCase().replace(/\s+/g, '').replace(/hz$/, '');
  const m = s.match(/^([0-9]+(?:[.,][0-9]+)?)([kmg]?)$/);
  if (!m) {
    // Tausenderpunkte wie "145.500.000"
    const plain = s.replace(/\./g, '');
    return /^[0-9]+$/.test(plain) ? parseInt(plain, 10) : null;
  }
  const v = parseFloat(m[1].replace(',', '.'));
  const mult = { '': v < 2000 ? 1e6 : 1, k: 1e3, m: 1e6, g: 1e9 }[m[2] as '' | 'k' | 'm' | 'g'];
  return Math.round(v * mult);
}

/** Achsenbeschriftung: sinnvolle Rasterweite für eine Spannweite und Pixelbreite. */
export function niceStep(span: number, px: number, minPx = 90): number {
  const raw = (span / px) * minPx;
  const p = Math.pow(10, Math.floor(Math.log10(raw)));
  for (const m of [1, 2, 2.5, 5, 10]) if (p * m >= raw) return p * m;
  return p * 10;
}
