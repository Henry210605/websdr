import { AudioOut } from './audio';
import { Connection } from './connection';
import type { Bookmark, Mode, ServerState } from './types';
import { MODES } from './types';
import type { PaletteName } from './waterfall';

/** Lokale Vorlieben, die nur in diesem Browser gespeichert werden. */
interface Prefs {
  step: number;
  snap: boolean;
  minDb: number;
  maxDb: number;
  palette: PaletteName;
  volume: number;
  stepPerMode: Partial<Record<Mode, number>>;
}

const PREFS_KEY = 'websdr.prefs';
const defaults: Prefs = {
  step: 100_000,
  snap: true,
  minDb: -100,
  maxDb: -30,
  palette: 'sdrsharp',
  volume: 0.8,
  stepPerMode: {},
};

function loadPrefs(): Prefs {
  try {
    return { ...defaults, ...JSON.parse(localStorage.getItem(PREFS_KEY) ?? '{}') };
  } catch {
    return { ...defaults };
  }
}

export const conn = new Connection();
export const audio = new AudioOut();

export const ui = $state({
  open: false,
  server: null as ServerState | null,
  error: '',
  level: -150,
  // Abstimmung (lokal sofort, Server bestätigt)
  vfo: 100_000_000,
  center: 99_700_000,
  mode: 'wfm' as Mode,
  bw: 200_000,
  squelch: null as number | null,
  // Ansicht
  zoom: [0, 1] as [number, number],
  view: { start: 0, end: 0 },
  audioOn: false,
  muted: false,
  bookmarks: [] as Bookmark[],
  ...loadPrefs(),
});

export function savePrefs() {
  const p: Prefs = {
    step: ui.step,
    snap: ui.snap,
    minDb: ui.minDb,
    maxDb: ui.maxDb,
    palette: ui.palette,
    volume: ui.volume,
    stepPerMode: ui.stepPerMode,
  };
  try {
    localStorage.setItem(PREFS_KEY, JSON.stringify(p));
  } catch {
    /* privater Modus o. Ä. – dann eben ohne Speichern */
  }
}

// --- Abstimmen mit Bündelung, damit schnelles Scrollen den Tuner nicht überflutet ---

let lastLocalTune = 0;
let lastLocalCenter = 0;
let tuneTimer: number | undefined;
let centerTimer: number | undefined;

export function tune(hz: number, snap = ui.snap) {
  let f = snap ? Math.round(hz / ui.step) * ui.step : Math.round(hz);
  f = Math.min(Math.max(f, 100_000), 1_766_000_000);
  ui.vfo = f;
  lastLocalTune = performance.now();
  if (tuneTimer === undefined) {
    tuneTimer = window.setTimeout(() => {
      tuneTimer = undefined;
      conn.send({ cmd: 'tune', vfo: ui.vfo });
    }, 30);
  }
}

export function setCenter(hz: number) {
  ui.center = Math.min(Math.max(Math.round(hz), 100_000), 1_766_000_000);
  lastLocalCenter = performance.now();
  if (centerTimer === undefined) {
    centerTimer = window.setTimeout(() => {
      centerTimer = undefined;
      conn.send({ cmd: 'center', freq: ui.center });
    }, 40);
  }
}

export function setMode(mode: Mode, bw?: number) {
  ui.stepPerMode[ui.mode] = ui.step;
  ui.mode = mode;
  ui.step = ui.stepPerMode[mode] ?? MODES.find((m) => m.id === mode)!.step;
  savePrefs();
  conn.send({ cmd: 'mode', mode, bw });
}

export function setBandwidth(bw: number) {
  ui.bw = Math.round(bw);
  conn.send({ cmd: 'bandwidth', bw: ui.bw });
}

export function setSquelch(db: number | null) {
  ui.squelch = db;
  conn.send({ cmd: 'squelch', db });
}

export function setZoom(a: number, b: number) {
  ui.zoom = [a, b];
  sendView();
}

let viewWidth = 1024;
export function setViewWidth(w: number) {
  viewWidth = Math.max(64, Math.min(8192, Math.round(w)));
  sendView();
}

function sendView() {
  conn.send({ cmd: 'view', width: viewWidth, zoom: ui.zoom });
}

export async function toggleAudio() {
  if (ui.audioOn) {
    ui.audioOn = false;
    conn.send({ cmd: 'audio', on: false });
    await audio.stop();
  } else {
    await audio.start();
    audio.setVolume(ui.volume);
    ui.audioOn = true;
    conn.send({ cmd: 'audio', on: true });
  }
}

/** Frequenzbereich, den der Kanalfilter belegt (für die Anzeige). */
export function filterRange(vfo = ui.vfo, mode = ui.mode, bw = ui.bw): [number, number] {
  switch (mode) {
    case 'usb':
      return [vfo + 100, vfo + bw];
    case 'lsb':
      return [vfo - bw, vfo - 100];
    default:
      return [vfo - bw / 2, vfo + bw / 2];
  }
}

// --- Lesezeichen ---

export async function loadBookmarks() {
  try {
    const r = await fetch('api/bookmarks');
    ui.bookmarks = await r.json();
  } catch {
    ui.bookmarks = [];
  }
}

export async function saveBookmarks() {
  await fetch('api/bookmarks', {
    method: 'PUT',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(ui.bookmarks),
  });
}

export function gotoBookmark(b: Bookmark) {
  if (b.mode !== ui.mode || b.bw !== ui.bw) {
    ui.mode = b.mode;
    ui.bw = b.bw;
    conn.send({ cmd: 'mode', mode: b.mode, bw: b.bw });
  }
  tune(b.freq, false);
}

// --- Serverereignisse ---

conn.onOpenChange = (open) => (ui.open = open);
conn.onError = (msg) => {
  ui.error = msg;
  setTimeout(() => {
    if (ui.error === msg) ui.error = '';
  }, 5000);
};
conn.onConnected = () => {
  sendView();
  if (ui.audioOn) conn.send({ cmd: 'audio', on: true });
};
conn.onState = (s) => {
  ui.server = s;
  const now = performance.now();
  const you = s.you.settings;
  if (now - lastLocalTune > 500) ui.vfo = you.vfo_hz;
  if (now - lastLocalCenter > 500) ui.center = s.device.center_hz;
  ui.mode = you.mode;
  ui.bw = you.bandwidth_hz;
  ui.squelch = you.squelch_db;
};
let lastLevelUpdate = 0;
conn.onAudio = (pcm, level) => {
  // S-Meter höchstens ~15× pro Sekunde aktualisieren
  const now = performance.now();
  if (now - lastLevelUpdate > 66) {
    lastLevelUpdate = now;
    ui.level = level;
  }
  if (ui.audioOn) audio.push(pcm);
};
