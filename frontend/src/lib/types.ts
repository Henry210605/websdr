export type Mode = 'am' | 'nfm' | 'wfm' | 'usb' | 'lsb' | 'cw' | 'dsb';

export interface DeviceSettings {
  center_hz: number;
  sample_rate: number;
  gain_db: number | null;
  ppm: number;
  bias_tee: boolean;
  fft_size: number;
}

export interface ClientSettings {
  vfo_hz: number;
  mode: Mode;
  bandwidth_hz: number;
  squelch_db: number | null;
}

export interface ServerState {
  type: 'state';
  device: DeviceSettings;
  source: {
    spec: string;
    connected: boolean;
    message: string;
    name: string;
    gains: number[];
    dropped_blocks: number;
  };
  you: { id: number; controller: boolean; settings: ClientSettings; audio_on: boolean };
  control_taken: boolean;
  password_required: boolean;
  clients: number;
  sample_rates: number[];
  fft_sizes: number[];
}

export interface Bookmark {
  name: string;
  freq: number;
  mode: Mode;
  bw: number;
}

export const MODES: { id: Mode; label: string; step: number }[] = [
  { id: 'nfm', label: 'NFM', step: 12_500 },
  { id: 'wfm', label: 'WFM', step: 100_000 },
  { id: 'am', label: 'AM', step: 5_000 },
  { id: 'usb', label: 'USB', step: 100 },
  { id: 'lsb', label: 'LSB', step: 100 },
  { id: 'cw', label: 'CW', step: 10 },
  { id: 'dsb', label: 'DSB', step: 1_000 },
];

export const STEPS = [1, 10, 100, 500, 1_000, 2_500, 5_000, 6_250, 8_330, 9_000, 10_000, 12_500, 25_000, 50_000, 100_000, 200_000, 1_000_000];

/** 8-Bit-Spektrumwerte des Servers → dBFS */
export const DB_MIN = -150;
export const DB_MAX = 0;
export const byteToDb = (v: number) => DB_MIN + (v * (DB_MAX - DB_MIN)) / 255;
export const dbToByte = (db: number) => ((db - DB_MIN) * 255) / (DB_MAX - DB_MIN);
