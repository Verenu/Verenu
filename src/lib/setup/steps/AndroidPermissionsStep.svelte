<script lang="ts">
	import { onMount } from 'svelte';
	import { invoke, listen } from '../../tauri';
	import { isAndroid } from '../../platform';
	import {
		ANDROID_PERMISSION_ORDER,
		FALLBACK_RATIONALE,
		isFunctional,
		missingRequired,
		parseGrant,
		recoveryCopy,
		type AndroidPermissionId,
		type AndroidPermissionSnapshot,
		type PermissionGrant,
	} from '../../android/permissions';

	// Android runtime-permission onboarding. Mirrors the macOS PermissionsStep
	// contract (`bind:allCoreGranted` gates the wizard's Continue button) but
	// for the four Android gates: microphone, accessibility service, battery
	// exemption, and notifications. Rationale copy matches the backend
	// (`android_permission_rationale`) so the wizard and Settings agree.
	let { allCoreGranted = $bindable(false) }: { allCoreGranted?: boolean } = $props();

	let snapshot = $state<AndroidPermissionSnapshot>({
		microphone: 'not_asked',
		accessibility_service: 'not_asked',
		battery_exemption: 'not_asked',
		notifications: 'not_asked',
	});
	let rationale = $state<Record<AndroidPermissionId, string>>({ ...FALLBACK_RATIONALE });
	let requesting = $state<AndroidPermissionId | null>(null);
	let note = $state('');

	const functional = $derived(isFunctional(snapshot));
	const missing = $derived(missingRequired(snapshot));

	$effect(() => {
		allCoreGranted = functional;
	});

	async function refresh() {
		note = '';
		try {
			const entries = await invoke<Array<{ id: string; rationale: string }>>(
				'android_permission_rationale',
			);
			for (const entry of entries) {
				const id = entry.id as AndroidPermissionId;
				if (id in rationale && entry.rationale) rationale[id] = entry.rationale;
			}
		} catch {
			// Backend unreachable (browser dev): keep fallback copy.
		}
		try {
			const nativeSnapshot = await invoke<Partial<Record<AndroidPermissionId, string>>>(
				'android_read_permissions',
			);
			updateSnapshot(nativeSnapshot);
		} catch {
			// The browser preview has no Android permission manager.
		}
		try {
			const result = await invoke<{ functional: boolean }>('android_evaluate_permissions', {
				snapshot,
			});
			allCoreGranted = result.functional;
		} catch {
			allCoreGranted = functional;
		}
	}

	async function request(permission: AndroidPermissionId) {
		if (requesting) return;
		requesting = permission;
		note = '';
		try {
			await invoke('android_request_permission', { permission });
			// Kotlin answers asynchronously via verenu:android-permission-snapshot.
			// If it never arrives (desktop dev), the row keeps its recovery copy.
		} catch {
			note = 'Could not open the system prompt. Use the Settings shortcut below instead.';
		} finally {
			requesting = null;
		}
	}

	function updateSnapshot(payload: Partial<Record<AndroidPermissionId, string>>) {
		for (const id of ANDROID_PERMISSION_ORDER) {
			const raw = payload[id];
			if (typeof raw === 'string') snapshot[id] = parseGrant(raw);
		}
	}

	function applySnapshot(payload: Partial<Record<AndroidPermissionId, string>>) {
		updateSnapshot(payload);
		void refresh();
	}

	onMount(() => {
		let disposed = false;
		const unlisteners: Array<() => void> = [];
		void refresh();
		const refreshOnReturn = () => {
			if (document.visibilityState === 'visible') void refresh();
		};
		window.addEventListener('focus', refreshOnReturn);
		document.addEventListener('visibilitychange', refreshOnReturn);
		Promise.all([
			listen<Partial<Record<AndroidPermissionId, string>>>(
				'verenu:android-permission-snapshot',
				(event) => {
					if (!disposed) applySnapshot(event.payload ?? {});
				},
			),
			listen<{ permission: string }>('verenu:android-permission-revoked', (event) => {
				const id = event.payload?.permission as AndroidPermissionId | undefined;
				if (!disposed && id && id in snapshot) {
					snapshot[id] = 'denied' as PermissionGrant;
					void refresh();
				}
			}),
		])
			.then((stops) => {
				if (disposed) stops.forEach((stop) => stop());
				else unlisteners.push(...stops);
			})
			.catch(() => {});
		return () => {
			disposed = true;
			window.removeEventListener('focus', refreshOnReturn);
			document.removeEventListener('visibilitychange', refreshOnReturn);
			unlisteners.forEach((stop) => stop());
		};
	});

	// Desktop/browser preview: lets QA walk the wizard without a device.
	function simulateGranted() {
		snapshot = {
			microphone: 'granted',
			accessibility_service: 'granted',
			battery_exemption: 'granted',
			notifications: 'granted',
		};
		void refresh();
	}

	function permissionLabel(id: AndroidPermissionId): string {
		switch (id) {
			case 'microphone':
				return 'Microphone';
			case 'accessibility_service':
				return 'Accessibility service';
			case 'battery_exemption':
				return 'Battery optimization';
			case 'notifications':
				return 'Notifications';
		}
	}

	function grantLabel(grant: PermissionGrant): string {
		switch (grant) {
			case 'granted':
				return 'On';
			case 'denied':
				return 'Off';
			case 'permanently_denied':
				return 'Blocked';
			case 'not_asked':
				return 'Not asked';
		}
	}

	function actionLabel(id: AndroidPermissionId, grant: PermissionGrant): string {
		if (id === 'accessibility_service' || id === 'battery_exemption') return 'Open Settings';
		if (grant === 'permanently_denied') return 'Open Settings';
		return 'Allow';
	}

	function targetLabel(id: AndroidPermissionId): string {
		switch (id) {
			case 'microphone':
			case 'notifications':
				return 'Android permission';
			case 'accessibility_service':
				return 'Accessibility settings';
			case 'battery_exemption':
				return 'Battery settings';
		}
	}
</script>

<div class="android-perms">
	{#if !isAndroid}
		<p class="preview-note" role="note">
			Android permission preview. On a device, each row opens its Android prompt or Settings page.
			<button class="btn-ghost btn-compact ui-focus-ring" onclick={simulateGranted}>
				Simulate granted
			</button>
		</p>
	{/if}

	<ol class="perm-list">
		{#each ANDROID_PERMISSION_ORDER as id (id)}
			{@const grant = snapshot[id]}
			{@const required = id === 'microphone' || id === 'accessibility_service'}
			<li class="perm-row" class:granted={grant === 'granted'}>
				<div class="perm-head">
					<span class="perm-name">{permissionLabel(id)}</span>
					<span class="perm-badges">
						{#if required}<span class="perm-req">Required</span>{/if}
						<span class="perm-state" data-grant={grant}>{grantLabel(grant)}</span>
					</span>
				</div>
				<p class="perm-why">{rationale[id]}</p>
				{#if grant !== 'granted'}
					<p class="perm-recovery">{recoveryCopy(id, grant)}</p>
					<div class="perm-actions">
						<button
							class="btn-primary btn-compact ui-focus-ring"
							disabled={requesting === id}
							onclick={() => request(id)}
						>
							{requesting === id ? 'Opening...' : actionLabel(id, grant)}
						</button>
						<span class="perm-target-hint">{targetLabel(id)}</span>
					</div>
				{/if}
			</li>
		{/each}
	</ol>

	{#if note}<p class="perm-note" role="status">{note}</p>{/if}
	{#if missing.length > 0}
		<p class="perm-blocked" role="status">
			Dictation needs {missing.length === 2 ? 'microphone and accessibility access' : missing[0] === 'microphone' ? 'microphone access' : 'the accessibility service'} before you can continue.
		</p>
	{:else}
		<p class="perm-ready" role="status">All set — dictation will work as soon as setup finishes.</p>
	{/if}
</div>

<style>
	.android-perms {
		display: flex;
		flex-direction: column;
		gap: 12px;
		width: 100%;
		max-width: 560px;
	}

	.preview-note {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 10px;
		font-size: 12px;
		color: var(--ink-mute);
		background: var(--paper-2);
		border: 1px solid var(--line);
		border-radius: var(--r-sm);
		padding: 8px 12px;
		margin: 0;
	}

	.perm-list {
		list-style: none;
		margin: 0;
		padding: 0;
		display: flex;
		flex-direction: column;
		gap: 8px;
	}

	.perm-row {
		border: 1px solid var(--line);
		border-radius: var(--r-md);
		background: var(--bg-elev);
		padding: 12px 14px;
		display: flex;
		flex-direction: column;
		gap: 6px;
	}

	.perm-row.granted {
		border-color: var(--success-line);
	}

	.perm-head {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 8px;
	}

	.perm-name {
		font-weight: 650;
		font-size: 13.5px;
		color: var(--ink);
	}

	.perm-badges {
		display: flex;
		align-items: center;
		gap: 6px;
	}

	.perm-req {
		font-size: 10.5px;
		font-weight: 700;
		letter-spacing: 0.04em;
		text-transform: uppercase;
		color: var(--accent-ink);
		background: var(--accent-soft);
		border-radius: 999px;
		padding: 2px 8px;
	}

	.perm-state {
		font-size: 11.5px;
		font-weight: 650;
		color: var(--ink-mute);
	}

	.perm-state[data-grant='granted'] {
		color: var(--success);
	}

	.perm-state[data-grant='permanently_denied'] {
		color: var(--danger);
	}

	.perm-why,
	.perm-recovery {
		margin: 0;
		font-size: 12.5px;
		line-height: 1.5;
		color: var(--ink-soft);
	}

	.perm-recovery {
		color: var(--ink-mute);
	}

	.perm-actions {
		display: flex;
		align-items: center;
		gap: 10px;
		margin-top: 2px;
	}

	.perm-target-hint {
		font-family: var(--mono);
		font-size: 10.5px;
		color: var(--ink-faint);
	}

	.perm-note,
	.perm-blocked,
	.perm-ready {
		margin: 0;
		font-size: 12.5px;
	}

	.perm-blocked {
		color: var(--warning);
	}

	.perm-ready {
		color: var(--success);
	}
</style>
