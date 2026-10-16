<script lang="ts">
	import { appStore } from '../../stores';
	import { icons } from '../../icons';

	const BASE = [
		{ id: 'home', label: 'Home', icon: 'home' },
		{ id: 'insights', label: 'Insights', icon: 'chart' },
	] as const;

	const LIBRARY = [{ id: 'contexts', label: 'Contexts', icon: 'layers' }] as const;
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

	function openSettings() {
		appStore.settingsMobileList = true;
		appStore.settingsOpen = true;
	}

	function go(page: (typeof items)[number]['id']) {
		appStore.settingsOpen = false;
		appStore.settingsMobileList = false;
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
		onclick={openSettings}
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
		gap: 4px;
		height: calc(68px + var(--safe-bottom, 0px));
		padding: 8px 8px calc(8px + var(--safe-bottom, 0px));
		padding-left: calc(8px + env(safe-area-inset-left, 0px));
		padding-right: calc(8px + env(safe-area-inset-right, 0px));
		background: var(--sidebar-bg);
		border-top: 1px solid var(--line);
		position: sticky;
		bottom: 0;
		z-index: 70;
	}

	.mobile-nav-item {
		flex: 1;
		min-width: 0;
		display: flex;
		flex-direction: column;
		align-items: center;
		justify-content: center;
		gap: 4px;
		padding: 0;
		background: transparent;
		border: 0;
		color: var(--ink-mute);
		font-size: 11px;
		font-weight: 500;
		letter-spacing: 0.01em;
		-webkit-tap-highlight-color: transparent;
	}

	.mobile-nav-icon {
		display: flex;
		align-items: center;
		justify-content: center;
		position: relative;
		width: 56px;
		height: 30px;
		border-radius: 999px;
		transition: color 180ms ease;
	}

	.mobile-nav-icon::before {
		content: '';
		position: absolute;
		inset: 0;
		border-radius: 999px;
		background: var(--control-active);
		opacity: 0;
		transform: scaleX(0.45);
		transition: transform 300ms cubic-bezier(0.22, 1, 0.36, 1), opacity 180ms ease;
	}

	.mobile-nav-icon svg {
		position: relative;
		transition: transform 260ms cubic-bezier(0.22, 1, 0.36, 1);
	}

	.mobile-nav-item.active .mobile-nav-icon svg {
		transform: translateY(-1px) scale(1.06);
	}

	.mobile-nav-item:active .mobile-nav-icon::before {
		opacity: 1;
		transform: scaleX(0.8);
		background: var(--control-hover);
	}

	.mobile-nav-label {
		line-height: 1;
		max-width: 100%;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.mobile-nav-item.active {
		color: var(--ink);
		font-weight: 600;
	}

	.mobile-nav-item.active .mobile-nav-icon::before {
		opacity: 1;
		transform: scaleX(1);
		background: var(--control-active);
	}

	@media (prefers-reduced-motion: reduce) {
		.mobile-nav-icon, .mobile-nav-icon::before, .mobile-nav-icon svg { transition: none; }
	}
</style>
