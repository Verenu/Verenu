<script lang="ts">
  import { slide } from 'svelte/transition';
  import { expoOut } from 'svelte/easing';
  import { MOTION_MS, motionMs } from '../../motion';
  import { formatDuration, type DiagnosticsSnapshot, type FailureEvent, type FailureGroup } from '../../diagnostics';
  import { ago, clockTime, formatFailures, subsystemLabel, whereLabel } from '../../diagnosticsInsights';

  interface Props {
    snapshot: DiagnosticsSnapshot;
    onCopy: (text: string, what: string) => void;
    onTrace: (traceId: string) => void;
    onLogs: (traceId: string) => void;
  }

  let { snapshot, onCopy, onTrace, onLogs }: Props = $props();

  let open = $state<string | null>(null);

  const groups = $derived([...snapshot.failure_groups].sort((a, b) => b.last_seen_ms - a.last_seen_ms));
  const events = $derived((fingerprint: string) => snapshot.latest_failures.filter((f) => f.fingerprint === fingerprint).slice(-6).reverse());

  function describe(group: FailureGroup, latest: FailureEvent | undefined): string {
    return [
      `${group.count}× ${subsystemLabel(group.subsystem)} ${whereLabel(group)}`,
      `cause: ${group.representative_cause}`,
      [group.provider, group.model].filter(Boolean).join('/') && `provider: ${[group.provider, group.model].filter(Boolean).join('/')}`,
      `fingerprint: ${group.fingerprint}`,
      group.last_trace_id && `trace: ${group.last_trace_id}`,
      latest?.duration_ms != null && `duration: ${formatDuration(latest.duration_ms)}`,
    ].filter(Boolean).join('\n');
  }
</script>

<div class="failures" data-setting-target="developer-latest-failures">
  <header class="dx-label">
    <span>Failure groups</span>
    {#if groups.length}<button class="link" onclick={() => onCopy(formatFailures(snapshot), 'Failures copied')}>Copy all</button>{:else}<b>0</b>{/if}
  </header>

  {#each groups as group, i (group.fingerprint)}
    {@const isOpen = open === group.fingerprint}
    {@const items = events(group.fingerprint)}
    <div class="group" class:open={isOpen}>
      <button class="head" aria-expanded={isOpen} onclick={() => (open = isOpen ? null : group.fingerprint)} style:--i={Math.min(i, 8)}>
        <span class="count">{group.count}×</span>
        <span class="main">
          <span class="where">{whereLabel(group)} <small>{subsystemLabel(group.subsystem)}</small></span>
          <span class="cause">{group.representative_cause}</span>
        </span>
        <span class="when">{ago(group.last_seen_ms)}</span>
      </button>
      {#if isOpen}
        <div class="more" transition:slide={{ duration: motionMs(MOTION_MS.panel), easing: expoOut }}>
          {#if group.provider || group.model}<p class="meta">{[group.provider, group.model].filter(Boolean).join(' · ')}</p>{/if}
          <p class="full">{group.representative_cause}</p>
          {#each items as item (item.id)}
            <div class="event">
              <time>{clockTime(item.timestamp_ms)}</time>
              <span>{item.error_category ?? ''}</span>
              <b>{formatDuration(item.duration_ms)}</b>
              {#if item.trace_id}<button class="link" onclick={() => onTrace(item.trace_id!)}>Trace</button><button class="link" onclick={() => onLogs(item.trace_id!)}>Logs</button>{:else}<span></span><span></span>{/if}
            </div>
          {/each}
          <div class="acts">
            <button class="btn-ghost btn-compact" onclick={() => onCopy(describe(group, items[0]), 'Failure copied')}>Copy for agent</button>
            <code>{group.fingerprint}</code>
          </div>
        </div>
      {/if}
    </div>
  {:else}
    <p class="dx-empty">No failures. Everything that ran, worked.</p>
  {/each}
</div>

<style>
  .failures { display:flex; flex-direction:column; }
  .group { border-bottom:1px solid var(--line-soft); }
  .head { all:unset; box-sizing:border-box; width:100%; display:grid; grid-template-columns:44px minmax(0,1fr) auto; gap:14px; align-items:center; padding:12px 6px; cursor:pointer; border-radius:var(--r-sm); animation:dx-in 320ms var(--ui-ease-out, ease) both; animation-delay:calc(var(--i) * 40ms); transition:background var(--ui-duration-fast, 150ms) ease; }
  .head:hover, .group.open .head { background:var(--control-hover); }
  .head:focus-visible { outline:2px solid var(--ink-strong); outline-offset:-2px; }
  .count { font:600 13px var(--mono); color:var(--danger); }
  .main { display:flex; flex-direction:column; gap:3px; min-width:0; }
  .where { font-size:13px; font-weight:500; color:var(--ink); }
  .where small { font-size:11px; font-weight:400; color:var(--ink-mute); margin-left:6px; }
  .cause { font-size:12px; color:var(--ink-mute); overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
  .when { font-size:11px; color:var(--ink-mute); white-space:nowrap; }

  .more { padding:2px 6px 16px 64px; }
  .meta { margin:0 0 4px; font:11px var(--mono); color:var(--ink-mute); }
  .full { margin:0 0 12px; font-size:12.5px; line-height:1.5; color:var(--ink-soft); overflow-wrap:anywhere; }
  .event { display:grid; grid-template-columns:78px minmax(0,1fr) 70px auto auto; gap:14px; align-items:center; padding:5px 0; font-size:11.5px; color:var(--ink-mute); }
  .event time, .event b { font:11px var(--mono); }
  .event b { color:var(--ink-soft); text-align:right; font-weight:400; }
  .acts { display:flex; align-items:center; gap:14px; margin-top:12px; }
  .acts code { margin-left:auto; font:10px var(--mono); color:var(--ink-faint); }
  .link { background:none; border:0; padding:2px 0; font:12px var(--sans); color:var(--ink-mute); cursor:pointer; transition:color var(--ui-duration-fast, 150ms) ease; }
  .link:hover { color:var(--ink); }

  @keyframes dx-in { from { opacity:0; transform:translateY(6px); } }
  @media (max-width: 760px) {
    .more { padding-left:6px; }
    .event { grid-template-columns:70px minmax(0,1fr) auto auto; }
    .event b { display:none; }
  }
  @media (prefers-reduced-motion: reduce) { .head { animation:none; } }
</style>
