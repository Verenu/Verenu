<script lang="ts">
	import { appStore } from '../../stores';

	// Compact-width bottom navigation for the Android shell. The desktop rail
	// (Sidebar.svelte) stays the navigation on medium/expanded windows; this
	// bar renders only when App.svelte decides the window is compact. Labels
	// mirror the rail's primary entries so the two never disagree.
	const items = [
		{ id: 'home', label: 'Home' },
		{ id: 'insights', label: 'Insights' },
		{ id: 'style', label: 'Style' },
	] as const;

	function go(page: (typeof items)[number]['id']) {
		appStore.settingsOpen = false;
		appStore.currentPage = page;
	}
</script>

<nav class="mobile-nav" aria-label="Primary">
	{#each items as item (item.id)}
		<button
			class="mobile-nav-item"
			class:active={appStore.currentPage === item.id && !appStore.settingsOpen}
			aria-current={appStore.currentPage === item.id && !appStore.settingsOpen ? 'page' : undefined}
			onclick={() => go(item.id)}
		>
			<span class="mobile-nav-dot" aria-hidden="true"></span>
			{item.label}
		</button>
	{/each}
	<button
		class="mobile-nav-item"
		class:active={appStore.settingsOpen}
		aria-current={appStore.settingsOpen ? 'page' : undefined}
		onclick={() => { appStore.settingsOpen = true; }}
	>
		<span class="mobile-nav-dot" aria-hidden="true"></span>
		Settings
	</button>
</nav>

<style>
	.mobile-nav {
		display: flex;
		gap: 4px;
		padding: 6px 10px calc(6px + env(safe-area-inset-bottom, 0px));
		padding-left: calc(10px + env(safe-area-inset-left, 0px));
		padding-right: calc(10px + env(safe-area-inset-right, 0px));
		background: var(--sidebar-bg);
		border-top: 1px solid var(--line);
		position: sticky;
		bottom: 0;
		z-index: 30;
	}

	.mobile-nav-item {
		flex: 1;
		min-height: var(--touch-target-min);
		display: flex;
		flex-direction: column;
		align-items: center;
		justify-content: center;
		gap: 3px;
		background: transparent;
		border: 0;
		border-radius: var(--r-sm);
		color: var(--ink-mute);
		font-size: 11px;
		font-weight: 600;
		letter-spacing: 0.01em;
	}

	.mobile-nav-item:active {
		background: var(--control-active);
	}

	.mobile-nav-dot {
		width: 4px;
		height: 4px;
		border-radius: 50%;
		background: transparent;
	}

	.mobile-nav-item.active {
		color: var(--ink);
	}

	.mobile-nav-item.active .mobile-nav-dot {
		background: var(--accent);
	}
</style>
