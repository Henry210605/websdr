<script lang="ts">
  import {
    conn,
    ui,
    audio,
    setMode,
    setBandwidth,
    setSquelch,
    savePrefs,
    gotoBookmark,
    saveBookmarks,
  } from '../lib/store.svelte';
  import { MODES, STEPS } from '../lib/types';
  import { formatBw, formatShort } from '../lib/format';

  const s = $derived(ui.server);
  const ctl = $derived(s?.you.controller ?? false);
  let password = $state('');
  let bmName = $state('');

  let open = $state<Record<string, boolean>>({ source: true, radio: true, display: true, audio: true, bookmarks: true });

  function stepLabel(v: number) {
    return v >= 1000 ? `${(v / 1000).toLocaleString('de-DE')} kHz` : `${v} Hz`;
  }

  function addBookmark() {
    const name = bmName.trim() || formatShort(ui.vfo, 4);
    ui.bookmarks = [...ui.bookmarks, { name, freq: ui.vfo, mode: ui.mode, bw: ui.bw }].sort((a, b) => a.freq - b.freq);
    bmName = '';
    saveBookmarks();
  }

  function removeBookmark(i: number) {
    ui.bookmarks = ui.bookmarks.filter((_, k) => k !== i);
    saveBookmarks();
  }

  function autoLevels() {
    // Rauschboden aus dem Spektrum schätzen: unteres Quantil der aktuellen Zeile
    const ev = new CustomEvent('websdr:autolevels');
    window.dispatchEvent(ev);
  }
</script>

<aside>
  <!-- Quelle -->
  <section>
    <button class="head" onclick={() => (open.source = !open.source)}>{open.source ? '▾' : '▸'} Quelle</button>
    {#if open.source && s}
      <div class="status" class:ok={s.source.connected}>
        <span class="dot"></span>
        {s.source.connected ? s.source.name || s.source.spec : s.source.message || 'nicht verbunden'}
      </div>
      {#if !ctl}
        <div class="note">
          {s.control_taken ? 'Ein anderer Browser steuert den Empfänger.' : 'Niemand steuert den Empfänger.'}
          {#if s.password_required}
            <input type="password" placeholder="Passwort" bind:value={password} />
          {/if}
          <button onclick={() => conn.send({ cmd: 'take_control', password: password || null })}>Steuerung übernehmen</button>
        </div>
      {/if}
      <label>
        Abtastrate
        <select
          disabled={!ctl}
          value={s.device.sample_rate}
          onchange={(e) => conn.send({ cmd: 'samplerate', sps: +e.currentTarget.value })}
        >
          {#each s.sample_rates as r (r)}
            <option value={r}>{(r / 1e6).toLocaleString('de-DE')} MS/s{r > 2_560_000 ? ' (instabil)' : ''}</option>
          {/each}
        </select>
      </label>
      <label class="row">
        <input
          type="checkbox"
          disabled={!ctl}
          checked={s.device.gain_db === null}
          onchange={(e) => conn.send({ cmd: 'gain', db: e.currentTarget.checked ? null : 29.7 })}
        />
        Automatische Verstärkung
      </label>
      {#if s.device.gain_db !== null}
        {@const gains = s.source.gains.length ? s.source.gains : [0, 49.6]}
        {@const idx = Math.max(0, gains.findIndex((g) => g >= (s.device.gain_db ?? 0) - 0.05))}
        <label>
          Verstärkung <b>{(s.device.gain_db ?? 0).toLocaleString('de-DE')} dB</b>
          <input
            type="range"
            min="0"
            max={gains.length - 1}
            value={idx}
            disabled={!ctl}
            oninput={(e) => conn.send({ cmd: 'gain', db: gains[+e.currentTarget.value] })}
          />
        </label>
      {/if}
      <div class="grid2">
        <label>
          PPM
          <input
            type="number"
            min="-200"
            max="200"
            value={s.device.ppm}
            disabled={!ctl}
            onchange={(e) => conn.send({ cmd: 'ppm', value: +e.currentTarget.value })}
          />
        </label>
        <label>
          FFT
          <select
            disabled={!ctl}
            value={s.device.fft_size}
            onchange={(e) => conn.send({ cmd: 'fft_size', size: +e.currentTarget.value })}
          >
            {#each s.fft_sizes as n (n)}<option value={n}>{n}</option>{/each}
          </select>
        </label>
      </div>
      <label class="row">
        <input
          type="checkbox"
          disabled={!ctl}
          checked={s.device.bias_tee}
          onchange={(e) => conn.send({ cmd: 'biastee', on: e.currentTarget.checked })}
        />
        Bias-Tee (Antennenspeisung 4,5 V)
      </label>
      {#if s.source.dropped_blocks > 0}
        <div class="warn">Verworfene Blöcke: {s.source.dropped_blocks} (CPU zu langsam?)</div>
      {/if}
    {/if}
  </section>

  <!-- Radio -->
  <section>
    <button class="head" onclick={() => (open.radio = !open.radio)}>{open.radio ? '▾' : '▸'} Radio</button>
    {#if open.radio}
      <div class="modes">
        {#each MODES as m (m.id)}
          <button class:active={ui.mode === m.id} onclick={() => setMode(m.id)}>{m.label}</button>
        {/each}
      </div>
      <label>
        Bandbreite <b>{formatBw(ui.bw)}</b>
        <input
          type="range"
          min={Math.log(50)}
          max={Math.log(250_000)}
          step="0.01"
          value={Math.log(ui.bw)}
          oninput={(e) => setBandwidth(Math.exp(+e.currentTarget.value))}
        />
      </label>
      <div class="grid2">
        <label>
          Schrittweite
          <select
            value={ui.step}
            onchange={(e) => {
              ui.step = +e.currentTarget.value;
              savePrefs();
            }}
          >
            {#each STEPS as v (v)}<option value={v}>{stepLabel(v)}</option>{/each}
          </select>
        </label>
        <label class="row snap">
          <input type="checkbox" bind:checked={ui.snap} onchange={savePrefs} />
          Raster
        </label>
      </div>
      <label class="row">
        <input
          type="checkbox"
          checked={ui.squelch !== null}
          onchange={(e) => setSquelch(e.currentTarget.checked ? Math.round(ui.level + 6) : null)}
        />
        Rauschsperre
        {#if ui.squelch !== null}<b>{ui.squelch} dB</b>{/if}
      </label>
      {#if ui.squelch !== null}
        <input type="range" min="-130" max="0" value={ui.squelch} oninput={(e) => setSquelch(+e.currentTarget.value)} />
      {/if}
    {/if}
  </section>

  <!-- Audio -->
  <section>
    <button class="head" onclick={() => (open.audio = !open.audio)}>{open.audio ? '▾' : '▸'} Audio</button>
    {#if open.audio}
      <label>
        Lautstärke <b>{Math.round(ui.volume * 100)} %</b>
        <input
          type="range"
          min="0"
          max="1"
          step="0.01"
          bind:value={ui.volume}
          oninput={() => {
            audio.setVolume(ui.volume);
            savePrefs();
          }}
        />
      </label>
      <label class="row">
        <input type="checkbox" bind:checked={ui.muted} onchange={() => audio.setMuted(ui.muted)} />
        Stumm
      </label>
    {/if}
  </section>

  <!-- Anzeige -->
  <section>
    <button class="head" onclick={() => (open.display = !open.display)}>{open.display ? '▾' : '▸'} Anzeige</button>
    {#if open.display}
      <label>
        Obergrenze <b>{ui.maxDb} dB</b>
        <input type="range" min="-120" max="0" bind:value={ui.maxDb} onchange={savePrefs} />
      </label>
      <label>
        Untergrenze <b>{ui.minDb} dB</b>
        <input type="range" min="-150" max="-20" bind:value={ui.minDb} onchange={savePrefs} />
      </label>
      <div class="grid2">
        <label>
          Farben
          <select bind:value={ui.palette} onchange={savePrefs}>
            <option value="sdrsharp">Klassisch</option>
            <option value="turbo">Turbo</option>
            <option value="grau">Graustufen</option>
          </select>
        </label>
        <button class="auto" onclick={autoLevels}>Auto-Pegel</button>
      </div>
    {/if}
  </section>

  <!-- Lesezeichen -->
  <section>
    <button class="head" onclick={() => (open.bookmarks = !open.bookmarks)}>{open.bookmarks ? '▾' : '▸'} Lesezeichen</button>
    {#if open.bookmarks}
      <div class="bmadd">
        <input placeholder="Name (optional)" bind:value={bmName} onkeydown={(e) => e.key === 'Enter' && addBookmark()} />
        <button onclick={addBookmark} title="Aktuelle Frequenz speichern">+</button>
      </div>
      <ul class="bms">
        {#each ui.bookmarks as b, i (i)}
          <li>
            <button class="bm" onclick={() => gotoBookmark(b)}>
              <span>{b.name}</span>
              <small>{formatShort(b.freq, 4)} · {b.mode.toUpperCase()}</small>
            </button>
            <button class="del" onclick={() => removeBookmark(i)} title="Löschen">×</button>
          </li>
        {:else}
          <li class="empty">Noch keine Lesezeichen</li>
        {/each}
      </ul>
    {/if}
  </section>
</aside>

<style>
  aside {
    overflow-y: auto;
    background: var(--panel);
    border-right: 1px solid var(--border);
    padding: 6px 10px 20px;
    font-size: 13px;
  }
  section {
    border-bottom: 1px solid var(--border);
    padding: 4px 0 8px;
  }
  .head {
    all: unset;
    cursor: pointer;
    display: block;
    width: 100%;
    font-weight: 600;
    padding: 6px 0;
    color: var(--text);
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 3px;
    margin: 6px 0;
    color: var(--muted);
  }
  label b {
    color: var(--text);
    font-weight: 500;
  }
  label.row {
    flex-direction: row;
    align-items: center;
    gap: 6px;
  }
  .grid2 {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px;
    align-items: end;
  }
  .snap {
    margin-bottom: 10px;
  }
  .modes {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 4px;
    margin: 4px 0 8px;
  }
  .modes button.active {
    background: var(--accent);
    color: #fff;
    border-color: var(--accent);
  }
  .status {
    display: flex;
    gap: 6px;
    align-items: center;
    color: #f99;
    margin: 4px 0;
  }
  .status.ok {
    color: #9f9;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: currentColor;
    flex: none;
  }
  .note {
    background: var(--panel2);
    padding: 6px;
    border-radius: 6px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin: 6px 0;
  }
  .warn {
    color: #fc6;
  }
  .auto {
    margin-bottom: 6px;
  }
  .bmadd {
    display: flex;
    gap: 4px;
  }
  .bmadd input {
    flex: 1;
    min-width: 0;
  }
  .bms {
    list-style: none;
    padding: 0;
    margin: 6px 0 0;
  }
  .bms li {
    display: flex;
    gap: 2px;
  }
  .bm {
    flex: 1;
    text-align: left;
    display: flex;
    flex-direction: column;
    background: transparent;
    border-color: transparent;
  }
  .bm:hover {
    background: var(--hover);
  }
  .bm small {
    color: var(--muted);
  }
  .del {
    background: transparent;
    border-color: transparent;
    color: var(--muted);
  }
  .empty {
    color: var(--muted);
    padding: 4px 0;
  }
</style>
