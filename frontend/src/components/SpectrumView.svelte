<script lang="ts">
  import { onMount } from 'svelte';
  import { conn, ui, tune, setCenter, setZoom, setViewWidth, setBandwidth, filterRange, savePrefs } from '../lib/store.svelte';
  import { Waterfall } from '../lib/waterfall';
  import { formatShort, niceStep } from '../lib/format';
  import { byteToDb, dbToByte } from '../lib/types';

  let wrap: HTMLDivElement;
  let specCanvas: HTMLCanvasElement;
  let wfCanvas: HTMLCanvasElement;
  let waterfall: Waterfall | null = null;
  let glError = $state('');

  // Letzte Spektrumzeile (geglättet) und Darstellungsbereich
  let row: Float32Array = new Float32Array(0);
  let rowStart = 0;
  let rowEnd = 0;
  let dirty = false;
  let hover = $state<{ x: number; y: number; f: number } | null>(null);

  const AXIS_H = 18;
  const SMOOTH = 0.45;

  const isController = $derived(ui.server?.you.controller ?? false);

  function xToFreq(x: number, width: number) {
    return ui.view.start + (x / width) * (ui.view.end - ui.view.start);
  }
  function freqToX(f: number, width: number) {
    return ((f - ui.view.start) / (ui.view.end - ui.view.start)) * width;
  }

  onMount(() => {
    try {
      waterfall = new Waterfall(wfCanvas);
      waterfall.setPalette(ui.palette);
    } catch (e) {
      glError = String(e);
    }

    conn.onSpectrum = (start, end, data) => {
      const sameView = data.length === row.length && start === rowStart && end === rowEnd;
      if (!sameView) row = new Float32Array(data.length);
      for (let i = 0; i < data.length; i++) {
        const db = byteToDb(data[i]);
        row[i] = sameView ? row[i] * SMOOTH + db * (1 - SMOOTH) : db;
      }
      rowStart = start;
      rowEnd = end;
      ui.view.start = start;
      ui.view.end = end;
      waterfall?.addRow(start, end, data);
      dirty = true;
    };

    // Auto-Pegel: Rauschboden (unteres Quantil) und stärkstes Signal der aktuellen Zeile
    const autoLevels = () => {
      if (row.length === 0) return;
      const sorted = Float32Array.from(row).sort();
      const floor = sorted[Math.floor(sorted.length * 0.1)];
      const peak = sorted[sorted.length - 1];
      ui.minDb = Math.round(floor - 8);
      ui.maxDb = Math.round(Math.max(peak + 5, floor + 40));
      savePrefs();
    };
    window.addEventListener('websdr:autolevels', autoLevels);
    let autoDone = false;
    const origHandler = conn.onSpectrum;
    conn.onSpectrum = (start, end, data) => {
      origHandler(start, end, data);
      if (!autoDone && row.length > 0) {
        autoDone = true;
        let hasPrefs = false;
        try {
          hasPrefs = !!localStorage.getItem('websdr.prefs');
        } catch {
          /* kein Speicher verfügbar */
        }
        if (!hasPrefs) autoLevels();
      }
    };

    const ro = new ResizeObserver(() => resize());
    ro.observe(wrap);
    resize();

    let raf = 0;
    const loop = () => {
      raf = requestAnimationFrame(loop);
      if (!dirty) return;
      dirty = false;
      drawSpectrum();
      waterfall?.draw(ui.view.start, ui.view.end, dbToByte(ui.minDb), dbToByte(ui.maxDb));
    };
    raf = requestAnimationFrame(loop);
    return () => {
      cancelAnimationFrame(raf);
      ro.disconnect();
      window.removeEventListener('websdr:autolevels', autoLevels);
    };
  });

  // Neu zeichnen, wenn sich Abstimmung oder Darstellung ändern
  $effect(() => {
    void [ui.vfo, ui.bw, ui.mode, ui.minDb, ui.maxDb, ui.center];
    dirty = true;
  });
  $effect(() => {
    waterfall?.setPalette(ui.palette);
    dirty = true;
  });

  function resize() {
    const dpr = window.devicePixelRatio || 1;
    for (const c of [specCanvas, wfCanvas]) {
      c.width = Math.max(1, Math.round(c.clientWidth * dpr));
      c.height = Math.max(1, Math.round(c.clientHeight * dpr));
    }
    setViewWidth(Math.min(wfCanvas.width, 4096));
    dirty = true;
  }

  function drawSpectrum() {
    const ctx = specCanvas.getContext('2d')!;
    const dpr = window.devicePixelRatio || 1;
    const W = specCanvas.width;
    const H = specCanvas.height;
    const axis = AXIS_H * dpr;
    const plotH = H - axis;
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.fillStyle = '#0b0d14';
    ctx.fillRect(0, 0, W, H);
    if (ui.view.end <= ui.view.start) return;

    const yOf = (db: number) => plotH - ((db - ui.minDb) / (ui.maxDb - ui.minDb)) * plotH;
    ctx.font = `${11 * dpr}px system-ui, sans-serif`;

    // dB-Raster
    ctx.strokeStyle = 'rgba(255,255,255,0.07)';
    ctx.fillStyle = 'rgba(255,255,255,0.45)';
    ctx.lineWidth = 1;
    const dbStep = ui.maxDb - ui.minDb > 80 ? 20 : 10;
    for (let db = Math.ceil(ui.minDb / dbStep) * dbStep; db <= ui.maxDb; db += dbStep) {
      const y = Math.round(yOf(db)) + 0.5;
      ctx.beginPath();
      ctx.moveTo(0, y);
      ctx.lineTo(W, y);
      ctx.stroke();
      ctx.fillText(`${db}`, 4 * dpr, y - 3 * dpr);
    }

    // Frequenzraster und Achse
    const span = ui.view.end - ui.view.start;
    const step = niceStep(span, W / dpr);
    ctx.textAlign = 'center';
    for (let f = Math.ceil(ui.view.start / step) * step; f <= ui.view.end; f += step) {
      const x = Math.round(freqToX(f, W)) + 0.5;
      ctx.beginPath();
      ctx.moveTo(x, 0);
      ctx.lineTo(x, plotH);
      ctx.stroke();
      ctx.fillText(formatShort(f, Math.min(6, Math.max(1, Math.ceil(-Math.log10(step / 1e6) - 1e-9)))), x, H - 5 * dpr);
    }
    ctx.textAlign = 'left';

    // Kanalfilter
    const [flo, fhi] = filterRange();
    const x0 = freqToX(flo, W);
    const x1 = freqToX(fhi, W);
    ctx.fillStyle = 'rgba(80,140,255,0.18)';
    ctx.fillRect(x0, 0, Math.max(1, x1 - x0), plotH);
    ctx.strokeStyle = 'rgba(120,170,255,0.6)';
    ctx.beginPath();
    ctx.moveTo(x0, 0);
    ctx.lineTo(x0, plotH);
    ctx.moveTo(x1, 0);
    ctx.lineTo(x1, plotH);
    ctx.stroke();

    // Spektrumkurve (die Zeile passt zur aktuellen Ansicht, da sie diese definiert)
    if (row.length > 1) {
      const n = row.length;
      ctx.beginPath();
      for (let i = 0; i < n; i++) {
        const x = (i / (n - 1)) * W;
        const y = yOf(row[i]);
        if (i === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      }
      ctx.strokeStyle = '#e8f1ff';
      ctx.lineWidth = 1.2 * dpr;
      ctx.stroke();
      ctx.lineTo(W, plotH);
      ctx.lineTo(0, plotH);
      ctx.closePath();
      const g = ctx.createLinearGradient(0, 0, 0, plotH);
      g.addColorStop(0, 'rgba(70,150,255,0.55)');
      g.addColorStop(1, 'rgba(20,40,120,0.15)');
      ctx.fillStyle = g;
      ctx.fill();
    }

    // Mitte (Hardware-LO) gestrichelt, VFO rot
    const xc = freqToX(ui.center, W);
    ctx.setLineDash([4 * dpr, 4 * dpr]);
    ctx.strokeStyle = 'rgba(255,255,255,0.25)';
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(xc, 0);
    ctx.lineTo(xc, plotH);
    ctx.stroke();
    ctx.setLineDash([]);
    const xv = freqToX(ui.vfo, W);
    ctx.strokeStyle = '#ff3b3b';
    ctx.lineWidth = 1.5 * dpr;
    ctx.beginPath();
    ctx.moveTo(xv, 0);
    ctx.lineTo(xv, plotH);
    ctx.stroke();

    // Achsenhintergrund-Linie
    ctx.strokeStyle = 'rgba(255,255,255,0.15)';
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(0, plotH + 0.5);
    ctx.lineTo(W, plotH + 0.5);
    ctx.stroke();
  }

  // --- Maus und Touch ---------------------------------------------------------

  type Drag =
    | { kind: 'pending'; x: number; y: number }
    | { kind: 'pan'; startX: number; startCenter: number }
    | { kind: 'edge'; side: -1 | 1 };
  let drag: Drag | null = null;
  let cursor = $state('crosshair');

  function localX(e: PointerEvent | WheelEvent) {
    const r = wrap.getBoundingClientRect();
    return { x: e.clientX - r.left, y: e.clientY - r.top, w: r.width };
  }

  function edgeAt(x: number, w: number): -1 | 1 | 0 {
    const [flo, fhi] = filterRange();
    const tol = 5;
    if (Math.abs(freqToX(flo, w) - x) < tol) return -1;
    if (Math.abs(freqToX(fhi, w) - x) < tol) return 1;
    return 0;
  }

  function onPointerDown(e: PointerEvent) {
    if (e.button !== 0) return;
    const { x, y, w } = localX(e);
    wrap.setPointerCapture(e.pointerId);
    const edge = edgeAt(x, w);
    drag = edge ? { kind: 'edge', side: edge } : { kind: 'pending', x, y };
  }

  function onPointerMove(e: PointerEvent) {
    const { x, y, w } = localX(e);
    hover = { x, y, f: xToFreq(x, w) };
    if (!drag) {
      cursor = edgeAt(x, w) ? 'ew-resize' : 'crosshair';
      return;
    }
    if (drag.kind === 'pending' && Math.abs(x - drag.x) > 4) {
      drag = { kind: 'pan', startX: drag.x, startCenter: ui.center };
      cursor = isController ? 'grabbing' : 'not-allowed';
    }
    if (drag.kind === 'pan' && isController) {
      const hzPerPx = (ui.view.end - ui.view.start) / w;
      setCenter(drag.startCenter - (x - drag.startX) * hzPerPx);
    } else if (drag.kind === 'edge') {
      const f = xToFreq(x, w);
      let bw: number;
      if (ui.mode === 'usb') bw = f - ui.vfo;
      else if (ui.mode === 'lsb') bw = ui.vfo - f;
      else bw = 2 * Math.abs(f - ui.vfo);
      setBandwidth(Math.max(50, bw));
    }
  }

  function onPointerUp(e: PointerEvent) {
    const { x, w } = localX(e);
    if (drag?.kind === 'pending') tune(xToFreq(x, w));
    drag = null;
    cursor = 'crosshair';
  }

  function onWheel(e: WheelEvent) {
    e.preventDefault();
    const { x, w } = localX(e);
    const dir = e.deltaY < 0 ? 1 : -1;
    if (e.ctrlKey) {
      // Zoom um die Mausposition
      const [a, b] = ui.zoom;
      const m = a + (x / w) * (b - a);
      const width = Math.min(1, Math.max(1 / 64, (b - a) * (dir > 0 ? 0.8 : 1.25)));
      let na = m - (x / w) * width;
      na = Math.min(Math.max(na, 0), 1 - width);
      setZoom(na, na + width);
    } else if (e.shiftKey) {
      if (isController) setCenter(ui.center + dir * (ui.view.end - ui.view.start) * 0.05);
    } else {
      tune(ui.vfo + dir * ui.step);
    }
  }
</script>

<div
  class="view"
  bind:this={wrap}
  style:cursor
  onpointerdown={onPointerDown}
  onpointermove={onPointerMove}
  onpointerup={onPointerUp}
  onpointerleave={() => (hover = null)}
  onwheel={onWheel}
  role="application"
  aria-label="Spektrum und Wasserfall: Klick stimmt ab, Ziehen verschiebt die Mitte, Mausrad ändert die Frequenz, Strg+Mausrad zoomt"
>
  <canvas class="spec" bind:this={specCanvas}></canvas>
  <canvas class="wf" bind:this={wfCanvas}></canvas>
  {#if glError}
    <div class="glerr">Wasserfall nicht verfügbar: {glError}</div>
  {/if}
  {#if hover}
    <div class="tip" style:left="{hover.x + 12}px" style:top="{hover.y + 12}px">
      {formatShort(hover.f, 4)}
    </div>
  {/if}
  {#if ui.zoom[1] - ui.zoom[0] < 0.999}
    <button class="unzoom" onclick={() => setZoom(0, 1)} onpointerdown={(e) => e.stopPropagation()}>Zoom zurücksetzen</button>
  {/if}
</div>

<style>
  .view {
    position: relative;
    display: grid;
    grid-template-rows: 36% 1fr;
    height: 100%;
    min-height: 0;
    touch-action: none;
    user-select: none;
    background: #05060a;
  }
  canvas {
    width: 100%;
    height: 100%;
    display: block;
    min-height: 0;
  }
  .wf {
    border-top: 1px solid #222838;
  }
  .tip {
    position: absolute;
    pointer-events: none;
    background: rgba(10, 12, 20, 0.85);
    color: #dfe7ff;
    border: 1px solid #333a50;
    font: 12px ui-monospace, monospace;
    padding: 2px 6px;
    border-radius: 4px;
    white-space: nowrap;
  }
  .glerr {
    position: absolute;
    bottom: 10px;
    left: 10px;
    color: #f88;
    font-size: 13px;
  }
  .unzoom {
    position: absolute;
    top: 6px;
    right: 8px;
    font-size: 12px;
  }
</style>
