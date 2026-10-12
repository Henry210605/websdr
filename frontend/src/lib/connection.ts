import type { ServerState } from './types';

export type SpectrumHandler = (start: number, end: number, data: Uint8Array) => void;
export type AudioHandler = (pcm: Int16Array, levelDb: number) => void;

/** WebSocket-Verbindung zum Server mit automatischem Wiederverbinden. */
export class Connection {
  private ws: WebSocket | null = null;
  private closed = false;
  onState: (s: ServerState) => void = () => {};
  onError: (msg: string) => void = () => {};
  onOpenChange: (open: boolean) => void = () => {};
  onSpectrum: SpectrumHandler = () => {};
  onAudio: AudioHandler = () => {};
  /** Wird nach jedem (Wieder-)Verbinden aufgerufen, um Ansicht/Audio neu zu melden. */
  onConnected: () => void = () => {};

  connect() {
    const proto = location.protocol === 'https:' ? 'wss' : 'ws';
    const ws = new WebSocket(`${proto}://${location.host}/ws`);
    ws.binaryType = 'arraybuffer';
    this.ws = ws;
    ws.onopen = () => {
      this.onOpenChange(true);
      this.onConnected();
    };
    ws.onclose = () => {
      this.onOpenChange(false);
      if (!this.closed) setTimeout(() => this.connect(), 1500);
    };
    ws.onmessage = (ev) => {
      if (typeof ev.data === 'string') {
        const msg = JSON.parse(ev.data);
        if (msg.type === 'state') this.onState(msg as ServerState);
        else if (msg.type === 'error') this.onError(msg.message);
        return;
      }
      const buf = ev.data as ArrayBuffer;
      const type = new Uint8Array(buf, 0, 1)[0];
      const view = new DataView(buf);
      if (type === 1) {
        const start = view.getFloat64(8, true);
        const end = view.getFloat64(16, true);
        this.onSpectrum(start, end, new Uint8Array(buf, 24));
      } else if (type === 2) {
        const level = view.getFloat32(4, true);
        this.onAudio(new Int16Array(buf, 8), level);
      }
    };
  }

  send(cmd: Record<string, unknown>) {
    if (this.ws?.readyState === WebSocket.OPEN) this.ws.send(JSON.stringify(cmd));
  }

  close() {
    this.closed = true;
    this.ws?.close();
  }
}
