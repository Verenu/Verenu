<script lang="ts" module>
  import { invoke } from '../tauri';
  import { createIconCache } from '../iconCache';

  const siteIconCache = createIconCache();

  function normalizeHost(input: string): string {
    const trimmed = input.trim().toLowerCase();
    if (!trimmed) return '';
    const withoutScheme = trimmed.includes('://') ? trimmed.slice(trimmed.indexOf('://') + 3) : trimmed;
    const host = withoutScheme.split(/[/?#]/)[0]?.split('@').pop()?.split(':')[0] ?? '';
    return host.replace(/^\.+|\.+$/g, '');
  }

  function loadSiteIcon(domain: string): Promise<string | null> {
    const host = normalizeHost(domain);
    if (!host) return Promise.resolve(null);
    return siteIconCache.get(host, () => invoke<string | null>('get_site_icon', { domain: host }));
  }
</script>

<script lang="ts">
  let { domain, size = 16 }: { domain: string; size?: number } = $props();

  let dataUri = $state<string | null>(null);

  $effect(() => {
    dataUri = null;
    let cancelled = false;
    loadSiteIcon(domain).then((uri) => {
      if (!cancelled) dataUri = uri;
    });
    return () => {
      cancelled = true;
    };
  });
</script>

{#if dataUri}
  <img
    class="site-icon"
    src={dataUri}
    alt=""
    style="width: {size}px; height: {size}px;"
    onerror={() => (dataUri = null)}
  />
{:else}
  <span class="site-icon site-icon-fallback" style="width: {size}px; height: {size}px;" aria-hidden="true">
    <svg width={Math.round(size * 0.7)} height={Math.round(size * 0.7)} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"><circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3c2.2 2.5 3.3 5.5 3.3 9s-1.1 6.5-3.3 9c-2.2-2.5-3.3-5.5-3.3-9S9.8 5.5 12 3Z"/></svg>
  </span>
{/if}

<style>
  .site-icon {
    border-radius: 6px;
    flex: 0 0 auto;
    display: block;
    object-fit: contain;
  }
  .site-icon-fallback {
    display: grid;
    place-items: center;
    background: var(--bg-elev);
    color: var(--ink-mute);
    border: 1px solid var(--line);
  }
</style>
