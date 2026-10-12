<script lang="ts">
  import { onMount } from 'svelte';
  import FreqDisplay from './components/FreqDisplay.svelte';
  import Sidebar from './components/Sidebar.svelte';
  import SpectrumView from './components/SpectrumView.svelte';
  import { conn, ui, tune, setCenter, toggleAudio, loadBookmarks, audio } from './lib/store.svelte';
  import { MODES } from './lib/types';

  let sidebarOpen = $state(window.innerWidth > 800);
  const ctl = $derived(ui.server?.you.controller ?? false);
  // Pegelanzeige: -120 … 0 dBFS
  const meter = $derived(Math.min(1, Math.max(0, (ui.level + 120) / 120)));
  const squelchPos = $derived(ui.squelch === null ? null : Math.min(1, Math.max(0, (ui.squelch + 120) / 120)));

  onMount(() => {
    conn.connect();
    loadBookmarks();
    return () => conn.close();
  });

  function onKey(e: KeyboardEvent) {
    const t = e.target as HTMLElement;
    if (t.tagName === 'INPUT' || t.tagName === 'SELECT' || t.tagName === 'TEXTAREA') return;
    const big = e.shiftKey ? 10 : 1;
    switch (e.key) {
      case 'ArrowRight':
        tune(ui.vfo + ui.step * big);
        break;
      case 'ArrowLeft':
        tune(ui.vfo - ui.step * big);
        break;
      case 'ArrowUp':
        tune(ui.vfo + ui.step * 10 * big);
        break;
      case 'ArrowDown':
        tune(ui.vfo - ui.step * 10 * big);
        break;
      case 'm':
        ui.muted = !ui.muted;
        audio.setMuted(ui.muted);
        break;
      case ' ':
        toggleAudio();
        break;
      default:
        return;
    }
    e.preventDefault();
  }
</script>

<svelte:window onkeydown={onKey} />

<div class="app" class:nosidebar={!sidebarOpen}>
  <header>
    <button class="menu" onclick={() => (sidebarOpen = !sidebarOpen)} title="Seitenleiste ein/aus">☰</button>
    <button class="play" class:on={ui.audioOn} onclick={toggleAudio} title="Audio starten/stoppen (Leertaste)">
      {ui.audioOn ? '■ Stopp' : '▶ Start'}
    </button>
    <div class="vfo">
      <FreqDisplay value={ui.vfo} onchange={(f) => tune(f, false)} title="Empfangsfrequenz (VFO)" />
      <span class="mode">{MODES.find((m) => m.id === ui.mode)?.label}</span>
    </div>
    <div class="center" class:locked={!ctl}>
      <span>Mitte</span>
      <FreqDisplay
        value={ui.center}
        size="small"
        onchange={(f) => ctl && setCenter(f)}
        title={ctl ? 'Mittenfrequenz des Sticks' : 'Nur mit Steuerrecht änderbar'}
      />
    </div>
    <div class="meter" title="Signalpegel im Kanal">
      <div class="bar"><div class="fill" style:width="{meter * 100}%"></div>
        {#if squelchPos !== null}<div class="sq" style:left="{squelchPos * 100}%"></div>{/if}
      </div>
      <span>{ui.level > -149 ? ui.level.toFixed(0) : '–'} dBFS</span>
    </div>
    <div class="conn" class:ok={ui.open && ui.server?.source.connected} title={ui.server?.source.message ?? ''}>
      {#if !ui.open}Keine Verbindung zum Server…{:else if !ui.server?.source.connected}Kein Empfänger{:else}●{/if}
    </div>
  </header>
  {#if sidebarOpen}<Sidebar />{/if}
  <main>
    <SpectrumView />
  </main>
  {#if ui.error}<div class="toast">{ui.error}</div>{/if}
</div>

<style>
  .app {
    display: grid;
    grid-template-columns: 270px 1fr;
    grid-template-rows: auto 1fr;
    height: 100%;
  }
  .app.nosidebar {
    grid-template-columns: 1fr;
  }
  header {
    grid-column: 1 / -1;
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 6px 12px;
    background: var(--panel);
    border-bottom: 1px solid var(--border);
    flex-wrap: wrap;
  }
  main {
    min-height: 0;
    min-width: 0;
  }
  .menu {
    font-size: 18px;
    padding: 2px 8px;
  }
  .play {
    min-width: 84px;
    font-weight: 600;
  }
  .play.on {
    background: #8b2330;
    border-color: #b03040;
  }
  .vfo {
    display: flex;
    align-items: baseline;
    gap: 10px;
  }
  .mode {
    font-weight: 700;
    color: var(--muted);
  }
  .center {
    display: flex;
    flex-direction: column;
    font-size: 11px;
    color: var(--muted);
  }
  .center.locked {
    opacity: 0.6;
  }
  .meter {
    display: flex;
    align-items: center;
    gap: 8px;
    font: 12px ui-monospace, monospace;
    color: var(--muted);
    min-width: 200px;
  }
  .bar {
    position: relative;
    flex: 1;
    height: 10px;
    background: #0a0c12;
    border: 1px solid var(--border);
    border-radius: 3px;
    overflow: hidden;
  }
  .fill {
    height: 100%;
    background: linear-gradient(90deg, #2fa84f, #d6c63a 70%, #e0453a);
    transition: width 80ms linear;
  }
  .sq {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 2px;
    background: #fff;
  }
  .conn {
    margin-left: auto;
    color: #f88;
    font-size: 13px;
  }
  .conn.ok {
    color: #5d5;
  }
  .toast {
    position: fixed;
    bottom: 16px;
    left: 50%;
    transform: translateX(-50%);
    background: #4a1f25;
    border: 1px solid #a33;
    padding: 8px 14px;
    border-radius: 6px;
  }
  @media (max-width: 800px) {
    .app {
      grid-template-columns: 1fr;
      grid-template-rows: auto auto 1fr;
    }
    .app :global(aside) {
      max-height: 40vh;
    }
    .meter {
      min-width: 120px;
    }
  }
</style>
