/** Audio-Ausgabe über AudioWorklet (48 kHz, mono). */
export class AudioOut {
  private ctx: AudioContext | null = null;
  private node: AudioWorkletNode | null = null;
  private gain: GainNode | null = null;
  private volume = 0.8;
  private muted = false;
  bufferedSec = 0;

  get running() {
    return this.ctx?.state === 'running';
  }

  /** Muss aus einem Klick heraus aufgerufen werden (Autoplay-Regeln der Browser). */
  async start() {
    if (!this.ctx) {
      this.ctx = new AudioContext({ sampleRate: 48000, latencyHint: 'interactive' });
      await this.ctx.audioWorklet.addModule('audio-worklet.js');
      this.node = new AudioWorkletNode(this.ctx, 'sdr-player', { outputChannelCount: [1] });
      this.node.port.onmessage = (e) => (this.bufferedSec = e.data.buffered);
      this.gain = this.ctx.createGain();
      this.node.connect(this.gain).connect(this.ctx.destination);
      this.applyGain();
    }
    await this.ctx.resume();
  }

  async stop() {
    await this.ctx?.suspend();
  }

  push(pcm: Int16Array) {
    if (!this.node || pcm.length === 0) return;
    const f = new Float32Array(pcm.length);
    for (let i = 0; i < pcm.length; i++) f[i] = pcm[i] / 32768;
    this.node.port.postMessage(f, [f.buffer]);
  }

  setVolume(v: number) {
    this.volume = v;
    this.applyGain();
  }

  setMuted(m: boolean) {
    this.muted = m;
    this.applyGain();
  }

  private applyGain() {
    if (this.gain) this.gain.gain.value = this.muted ? 0 : this.volume * this.volume;
  }
}
