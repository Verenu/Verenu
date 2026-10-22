<script lang="ts">
  import Toggle from './Toggle.svelte';
  import { t3State } from '../t3Store.svelte';
  let { enabled, onchange, disabled = false }: { enabled: boolean; onchange: (enabled: boolean) => void; disabled?: boolean } = $props();
  const connection = $derived(t3State.status?.connection);
  const skills = $derived(t3State.status?.skills ?? []);
</script>

<section class="t3-setting" aria-label="T3 Code skill mentions">
  <div class="setting-row">
    <div><span class="label">T3 Code · Format spoken skill names</span><p>Say “use my babysit skill” to insert <code>$babysit-pr</code>. Applies only in T3 Code.</p></div>
    <Toggle checked={enabled} {disabled} {onchange} label="Format spoken skill names in T3 Code" />
  </div>
  <p>{connection ? `${skills.length} shared skills imported.` : 'Connect T3 Code in Settings → Integrations.'} Requires a working cleanup provider. Only skill names go to that provider.</p>
</section>

<style>
  .t3-setting { padding: 12px 0; border-top: 1px solid var(--line-soft); }
  .setting-row { display: flex; justify-content: space-between; align-items: center; gap: 16px; }
  .label { color: var(--ink-soft); font-size: 12px; font-weight: 500; }
  p { color: var(--ink-mute); font-size: 11px; line-height: 1.6; margin: 5px 0 0; }
  code { font-size: 11px; color: var(--ink-soft); }
</style>
