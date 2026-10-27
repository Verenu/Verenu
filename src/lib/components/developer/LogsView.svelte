<script lang="ts">
  import { fade, fly, slide } from 'svelte/transition';
  import { expoOut } from 'svelte/easing';
  import Dropdown from '../Dropdown.svelte';
  import { MOTION_MS, MOTION_PX, motionMs, motionPx } from '../../motion';
  import { filterLogs, formatDuration, type StructuredLogEntry } from '../../diagnostics';
  import { clockTime, formatLogs, subsystemLabel } from '../../diagnosticsInsights';

  interface Props {
    logs: StructuredLogEntry[];
    query: string;
    level: string;
    subsystem: string;
    trace: string;
    onCopy: (text: string, what: string) => void;
  }

  let { logs, query = $bindable(), level = $bindable(), subsystem = $bindable(), trace = $bindable(), onCopy }: Props = $props();

  const LEVELS = [
    { value: 'all', label: 'All' },
    { value: 'problems', label: 'Problems' },
    { value: 'error', label: 'Errors' },
  ];
  const RENDER_LIMIT = 400;

  let paused = $state(false);
  let frozen = $state<StructuredLogEntry[]>([]);
  let menuOpen = $state(false);
  let expanded = $state<string | null>(null);

  const source = $derived(paused ? frozen : logs);
  const visible = $derived(filterLogs(source, query, level, subsystem, trace));
  const subsystems = $derived([...new Set(source.map((entry) => entry.subsystem))].sort());

  interface Row { key: string; entry: StructuredLogEntry; repeat: number }
  const rows = $derived.by(() => {
    const out: Row[] = [];
    const seen = new Map<string, number>();
    for (const entry of visible.slice(-RENDER_LIMIT).reverse()) {
      const base = `${entry.timestamp_ms}|${entry.subsystem}|${entry.level}|${entry.message.slice(0, 80)}`;
      const n = seen.get(base) ?? 0;
      seen.set(base, n + 1);
      const key = `${base}#${n}`;
      const prev = out[out.length - 1];
      if (prev && prev.entry.message === entry.message && prev.entry.subsystem === entry.subsystem && prev.entry.level === entry.level) {
        prev.repeat += 1;
        prev.key = key;
      } else {
        out.push({ key, entry, repeat: 1 });
      }
    }
    return out;
  });

  function togglePause() {
    if (!paused) frozen = logs;
    paused = !paused;
  }
</script>

<div class="logs" data-setting-target="developer-log-stream">
  <div class="bar">
    <input aria-label="Search logs" placeholder="Search" bind:value={query} />
    <div class="seg" role="group" aria-label="Log level">
      {#each LEVELS as opt (opt.value)}
        <button class:active={level === opt.value} aria-pressed={level === opt.value} onclick={() => (level = opt.value)}>{opt.label}</button>
      {/each}
    </div>
    <Dropdown bind:open={menuOpen} closeSelector=".log-subsystem-dropdown">
      <div class="ui-dropdown log-subsystem-dropdown">
        <button class="ui-dropdown-trigger ui-dropdown-trigger--compact" aria-haspopup="true" aria-expanded={menuOpen} aria-label="Log subsystem" onclick={() => (menuOpen = !menuOpen)}>
          <span class="trigger-label">{subsystem === 'all' ? 'All subsystems' : subsystemLabel(subsystem)}</span>
          <svg class:open={menuOpen} width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m6 9 6 6 6-6" /></svg>
        </button>
        {#if menuOpen}
          <div class="ui-dropdown-menu ui-dropdown-menu--padded menu scroll-styled scroll-thumb-elev" role="listbox" aria-label="Log subsystem options" in:fly={{ y: -motionPx(MOTION_PX.nudge), duration: motionMs(MOTION_MS.panel), easing: expoOut }} out:fade={{ duration: motionMs(MOTION_MS.fast) }}>
            {#each [{ value: 'all', label: 'All subsystems' }, ...subsystems.map((s) => ({ value: s, label: subsystemLabel(s) }))] as opt (opt.value)}
              <button class="ui-dropdown-option" class:active={subsystem === opt.value} onclick={() => { subsystem = opt.value; menuOpen = false; }}>{opt.label}</button>
            {/each}
          </div>
        {/if}
      </div>
    </Dropdown>
    {#if trace}
      <button class="chip" transition:fade={{ duration: motionMs(MOTION_MS.fast) }} onclick={() => (trace = '')} title="Clear trace filter">{trace.slice(0, 18)} <span aria-hidden="true">✕</span></button>
    {/if}
    <span class="spacer"></span>
    <button class="btn-ghost btn-compact" onclick={togglePause}>{paused ? 'Resume' : 'Pause'}</button>
    <button class="btn-primary btn-compact" disabled={!visible.length} onclick={() => onCopy(formatLogs(visible.slice(-RENDER_LIMIT * 2)), `${Math.min(visible.length, RENDER_LIMIT * 2)} log lines copied`)}>Copy {visible.length}</button>
  </div>

  <div class="table" role="table" aria-label="Structured logs">
    {#each rows as row (row.key)}
      {@const entry = row.entry}
      {@const isOpen = expanded === row.key}
      <div class="row" data-level={entry.level} role="row">
        <button class="line" aria-expanded={isOpen} onclick={() => (expanded = isOpen ? null : row.key)}>
          <time>{clockTime(entry.timestamp_ms)}</time>
          <b class="lvl">{entry.level}</b>
          <span class="where">{subsystemLabel(entry.subsystem)}{entry.operation ?? entry.stage ? ` · ${entry.operation ?? entry.stage}` : ''}</span>
          <span class="msg" class:wrap={isOpen}>{entry.message}</span>
          {#if row.repeat > 1}<span class="rep">×{row.repeat}</span>{/if}
        </button>
        {#if isOpen}
          <div class="extra" transition:slide={{ duration: motionMs(MOTION_MS.base), easing: expoOut }}>
            {#if entry.duration_ms != null}<span>{formatDuration(entry.duration_ms)}</span>{/if}
            {#if entry.error_category}<span>{entry.error_category}</span>{/if}
            {#each Object.entries(entry.measurements ?? {}) as [k, v] (k)}<span>{k} {v}</span>{/each}
            {#if entry.trace_id}<button class="link" onclick={() => (trace = entry.trace_id ?? '')}>Filter to {entry.trace_id}</button>{/if}
            <button class="link" onclick={() => onCopy(formatLogs([entry]), 'Line copied')}>Copy line</button>
          </div>
        {/if}
      </div>
    {:else}
      <p class="dx-empty">{logs.length ? 'No lines match.' : 'No logs yet.'}</p>
    {/each}
  </div>
</div>

<style>
  .logs { display:flex; flex-direction:column; gap:14px; }
  .bar { display:flex; align-items:center; flex-wrap:wrap; gap:8px; }
  .spacer { flex:1; }
  input { flex:0 1 200px; min-width:120px; background:var(--bg-elev); border:1px solid var(--line); color:var(--ink); border-radius:var(--r-sm); padding:6px 9px; font:12px var(--sans); transition:border-color var(--ui-duration-fast, 150ms) ease; }
  input:focus-visible { outline:none; border-color:var(--line-strong); }
  .seg { display:inline-flex; border:1px solid var(--line); border-radius:var(--r-sm); overflow:hidden; }
  .seg button { background:none; border:0; padding:6px 11px; font:12px var(--sans); color:var(--ink-mute); cursor:pointer; transition:color var(--ui-duration-fast, 150ms) ease, background var(--ui-duration-fast, 150ms) ease; }
  .seg button + button { border-left:1px solid var(--line); }
  .seg button:hover { color:var(--ink); }
  .seg button.active { background:var(--control-active); color:var(--ink); }
  .seg button:focus-visible { outline:2px solid var(--ink-strong); outline-offset:-2px; }
  .trigger-label { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; max-width:150px; }
  .menu { max-height:260px; min-width:230px; }
  .chip { display:inline-flex; gap:6px; align-items:center; background:var(--accent-soft); border:1px solid var(--line); color:var(--ink-soft); border-radius:99px; padding:3px 10px; font:11px var(--mono); cursor:pointer; }
  .chip:hover { color:var(--ink); }

  .table { border-top:1px solid var(--line); }
  .row { border-bottom:1px solid var(--line-soft); border-left:2px solid transparent; }
  .row[data-level="error"] { border-left-color:var(--danger); }
  .row[data-level="warn"] { border-left-color:var(--warning); }
  .line { all:unset; box-sizing:border-box; width:100%; display:grid; grid-template-columns:72px 44px minmax(90px,170px) minmax(0,1fr) auto; gap:10px; align-items:baseline; padding:6px 8px; cursor:pointer; font:11.5px var(--mono); transition:background var(--ui-duration-fast, 150ms) ease; }
  .line:hover { background:var(--control-hover); }
  .line:focus-visible { outline:2px solid var(--ink-strong); outline-offset:-2px; }
  time, .where { color:var(--ink-mute); font-size:10.5px; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
  .lvl { font-weight:500; font-size:10.5px; text-transform:uppercase; color:var(--ink-faint); }
  .row[data-level="error"] .lvl { color:var(--danger); }
  .row[data-level="warn"] .lvl { color:var(--warning); }
  .msg { color:var(--ink-soft); overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
  .msg.wrap { white-space:normal; overflow-wrap:anywhere; }
  .rep { color:var(--ink-mute); font-size:10.5px; }
  .extra { display:flex; flex-wrap:wrap; gap:4px 16px; padding:2px 8px 10px 126px; font:10.5px var(--mono); color:var(--ink-mute); }
  .link { background:none; border:0; padding:0; font:11.5px var(--sans); color:var(--ink-soft); cursor:pointer; }
  .link:hover { color:var(--ink); text-decoration:underline; }

  @media (max-width: 760px) {
    .line { grid-template-columns:62px 38px minmax(0,1fr); }
    .where { display:none; }
    .extra { padding-left:8px; }
  }
</style>
