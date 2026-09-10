<script lang="ts">
	import { appStore } from '../../stores';
	import { icons } from '../../icons';

	// Compact-width bottom navigation for the Android shell. The desktop rail
	// (Sidebar.svelte) stays the navigation on medium/expanded windows; this
	// bar renders only when App.svelte decides the window is compact. Entries
	// mirror the rail's primary ones so the two never disagree.
	//
	// Icon-first: at phone widths a label per tab is more text than a bottom
	// bar can carry legibly, so the icon leads and the label sits under it at
	// caption size — the standard Android bottom-bar treatment.
	const BASE = [
		{ id: 'home', label: 'Home', icon: 'home' },
		{ id: 'insights', label: 'Insights', icon: 'chart' },
	] as const;

	/*
	 * Contexts (or the legacy Dictionary/Snippets pair it replaced) are reached
	 * from the sidebar rail on desktop. That rail is hidden here, so without
	 * these entries the whole surface is unreachable on a phone. Mirrors the
	 * same legacy switch App.svelte routes on.
	 */
	const LIBRARY = [{ id: 'contexts', label: 'Contexts', icon: 'apps' }] as const;
	const LEGACY_LIBRARY = [
		{ id: 'dictionary', label: 'Words', icon: 'book' },
		{ id: 'snippets', label: 'Snippets', icon: 'scissors' },
	] as const;
	const TAIL = [{ id: 'style', label: 'Style', icon: 'type' }] as const;

	const items = $derived([
		...BASE,
		...(appStore.legacyFeaturesEnabled ? LEGACY_LIBRARY : LIBRARY),
		...TAIL,
	]);

	function go(page: (typeof items)[number]['id']) {
		appStore.settingsOpen = false;
		appStore.currentPage = page;
	}
</script>

<nav class="mobile-nav" aria-label="Primary">
	{#each items as item (item.id)}
		{@const active = appStore.currentPage === item.id && !appStore.settingsOpen}
		<button
			class="mobile-nav-item"
			class:active
			aria-current={active ? 'page' : undefined}
			onclick={() => go(item.id)}
		>
			<span class="mobile-nav-icon" aria-hidden="true">
				<svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width={active ? '2.2' : '1.7'} stroke-linecap="round" stroke-linejoin="round">{@html icons[item.icon as keyof typeof icons]}</svg>
			</span>
			<span class="mobile-nav-label">{item.label}</span>
		</button>
	{/each}
	<button
		class="mobile-nav-item"
		class:active={appStore.settingsOpen}
		aria-current={appStore.settingsOpen ? 'page' : undefined}
		onclick={() => { appStore.settingsOpen = true; }}
	>
		<span class="mobile-nav-icon" aria-hidden="true">
			<svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width={appStore.settingsOpen ? '2.2' : '1.7'} stroke-linecap="round" stroke-linejoin="round">{@html icons.settings}</svg>
		</span>
		<span class="mobile-nav-label">Settings</span>
	</button>
</nav>

<style>
	.mobile-nav {
		display: flex;
		gap: 2px;
		padding: 4px 6px calc(4px + env(safe-area-inset-bottom, 0px));
		padding-left: calc(8px + env(safe-area-inset-left, 0px));
		padding-right: calc(8px + env(safe-area-inset-right, 0px));
		background: var(--sidebar-bg);
		border-top: 1px solid var(--line);
		position: sticky;
		bottom: 0;
		/* Above the settings overlay (z-index 60). Settings is a page on mobile,
		   not a dialog, and the sidebar's "Back to app" button that normally
		   closes it is hidden here — without this the opaque settings wash
		   paints over the bar and there is no way back out of settings. */
		z-index: 70;
	}

	.mobile-nav-item {
		flex: 1;
		min-height: var(--touch-target-min);
		display: flex;
		flex-direction: column;
		align-items: center;
		justify-content: center;
		gap: 3px;
		padding: 5px 1px;
		background: transparent;
		border: 0;
		border-radius: var(--r-sm);
		color: var(--ink-faint);
		font-size: 10px;
		font-weight: 500;
		letter-spacing: 0.01em;
	}

	.mobile-nav-item:active {
		background: var(--control-active);
	}

	.mobile-nav-icon {
		display: flex;
		align-items: center;
		justify-content: center;
	}

	.mobile-nav-label {
		line-height: 1;
	}

	.mobile-nav-item.active {
		color: var(--ink);
		font-weight: 600;
	}
</style>
