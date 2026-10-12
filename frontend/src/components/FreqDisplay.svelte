<script lang="ts">
  import { parseFreq } from '../lib/format';

  // Ziffernanzeige wie in SDR#: Mausrad über einer Ziffer ändert genau diese Stelle,
  // Klick in die obere/untere Hälfte zählt hoch/runter, Doppelklick öffnet die Eingabe.
  let {
    value,
    onchange,
    size = 'large',
    title = '',
  }: { value: number; onchange: (hz: number) => void; size?: 'large' | 'small'; title?: string } = $props();

  const DIGITS = 10; // bis 9.999.999.999 Hz
  let editing = $state(false);
  let text = $state('');
  let input: HTMLInputElement | undefined = $state();

  const digits = $derived.by(() => {
    const s = Math.max(0, Math.round(value)).toString().padStart(DIGITS, '0');
    const firstSignificant = s.search(/[1-9]/);
    return s.split('').map((ch, i) => ({
      ch,
      pow: DIGITS - 1 - i,
      dim: firstSignificant === -1 || i < firstSignificant,
      sep: i > 0 && (DIGITS - i) % 3 === 0,
    }));
  });

  function bump(pow: number, dir: number) {
    onchange(Math.max(0, value + dir * Math.pow(10, pow)));
  }

  function onWheel(e: WheelEvent, pow: number) {
    e.preventDefault();
    bump(pow, e.deltaY < 0 ? 1 : -1);
  }

  function onClick(e: MouseEvent, pow: number) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    bump(pow, e.clientY < r.top + r.height / 2 ? 1 : -1);
  }

  function startEdit() {
    text = (value / 1e6).toFixed(6).replace(/0+$/, '').replace(/\.$/, '');
    editing = true;
    queueMicrotask(() => input?.select());
  }

  function commit() {
    const f = parseFreq(text);
    if (f !== null) onchange(f);
    editing = false;
  }
</script>

<div class="freq {size}" {title}>
  {#if editing}
    <input
      bind:this={input}
      bind:value={text}
      onkeydown={(e) => {
        if (e.key === 'Enter') commit();
        if (e.key === 'Escape') editing = false;
      }}
      onblur={commit}
      aria-label="Frequenz eingeben (z. B. 145.5M oder 7100k)"
    />
  {:else}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="digits" ondblclick={startEdit}>
      {#each digits as d (d.pow)}
        {#if d.sep}<span class="sep" class:dim={d.dim}>.</span>{/if}
        <button
          class="digit"
          class:dim={d.dim}
          onwheel={(e) => onWheel(e, d.pow)}
          onclick={(e) => onClick(e, d.pow)}
          tabindex="-1">{d.ch}</button
        >
      {/each}
      <span class="unit">Hz</span>
      <button class="edit" onclick={startEdit} title="Frequenz eingeben">✎</button>
    </div>
  {/if}
</div>

<style>
  .freq {
    font-family: 'JetBrains Mono', 'Consolas', ui-monospace, monospace;
    color: var(--freq);
    user-select: none;
  }
  .digits {
    display: flex;
    align-items: baseline;
  }
  .large {
    font-size: 30px;
  }
  .small {
    font-size: 16px;
  }
  .digit {
    all: unset;
    cursor: ns-resize;
    padding: 0 0.02em;
    border-radius: 3px;
    font-variant-numeric: tabular-nums;
  }
  .digit:hover {
    background: var(--hover);
    color: #fff;
  }
  .dim {
    opacity: 0.25;
  }
  .sep {
    margin: 0 -0.05em;
  }
  .unit {
    font-size: 0.45em;
    margin-left: 0.4em;
    color: var(--muted);
  }
  .edit {
    all: unset;
    cursor: pointer;
    font-size: 0.45em;
    margin-left: 0.6em;
    color: var(--muted);
  }
  .edit:hover {
    color: var(--text);
  }
  input {
    font: inherit;
    width: 9em;
    background: var(--panel2);
    color: var(--freq);
    border: 1px solid var(--accent);
    border-radius: 4px;
    padding: 0 0.2em;
  }
</style>
