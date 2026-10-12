// Audio-Wiedergabe mit Jitter-Puffer. Läuft im Audio-Thread des Browsers.
// Erhält Float32-Blöcke (48 kHz) per port.postMessage.

class SdrPlayer extends AudioWorkletProcessor {
  constructor() {
    super();
    this.size = 48000 * 4; // 4 s Ringpuffer
    this.buf = new Float32Array(this.size);
    this.read = 0;
    this.write = 0;
    this.target = 48000 * 0.15; // Ziel: 150 ms Vorrat
    this.buffering = true;
    this.pos = 0; // fraktionale Leseposition für Driftausgleich
    this.port.onmessage = (e) => this.push(e.data);
    this.reportCounter = 0;
  }

  available() {
    return (this.write - this.read + this.size) % this.size;
  }

  push(data) {
    for (let i = 0; i < data.length; i++) {
      this.buf[this.write] = data[i];
      this.write = (this.write + 1) % this.size;
    }
    // Viel zu viel Vorrat (z. B. nach Tab im Hintergrund): auf Ziel zurückspringen.
    if (this.available() > this.target * 4) {
      this.read = (this.write - Math.floor(this.target) + this.size) % this.size;
    }
  }

  process(_inputs, outputs) {
    const out = outputs[0][0];
    const avail = this.available();
    if (this.buffering) {
      if (avail >= this.target) this.buffering = false;
      else {
        out.fill(0);
        return true;
      }
    }
    // Leichter Driftausgleich: bei zu viel Vorrat etwas schneller lesen, bei zu wenig langsamer.
    const ratio = avail > this.target * 1.5 ? 1.002 : avail < this.target * 0.5 ? 0.998 : 1.0;
    for (let i = 0; i < out.length; i++) {
      if (this.available() < 2) {
        this.buffering = true;
        out.fill(0, i);
        break;
      }
      const a = this.buf[this.read];
      const b = this.buf[(this.read + 1) % this.size];
      out[i] = a + (b - a) * this.pos;
      this.pos += ratio;
      while (this.pos >= 1) {
        this.pos -= 1;
        this.read = (this.read + 1) % this.size;
      }
    }
    if (++this.reportCounter % 100 === 0) this.port.postMessage({ buffered: this.available() / 48000 });
    return true;
  }
}

registerProcessor('sdr-player', SdrPlayer);
