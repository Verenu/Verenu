import { emit, invoke } from './tauri';
import { defaultHotkey, formatKeyLabel } from './platform';
import { desktopShortcut, loadDesktopShortcuts } from './shortcutStatus.svelte';

const state = $state<{ codes: string[] }>({ codes: defaultHotkey });

export function hotkeyCodes(): string[] {
  const desktop = desktopShortcut('dictation');
  return desktop ? desktop.codes : state.codes;
}

export function hotkeyLabels(): string[] {
  if (desktopShortcut('dictation')?.active === null) return ['Unavailable'];
	return hotkeyCodes().filter(Boolean).map(formatKeyLabel);
}

export async function loadHotkey(): Promise<void> {
	await loadDesktopShortcuts();
	try {
		const saved = await invoke<string[] | null>('get_setting', { key: 'hotkey' });
		if (Array.isArray(saved) && saved.length > 0 && saved.every((code) => typeof code === 'string') && saved.some(Boolean)) {
			state.codes = saved.filter(Boolean);
			await emit('verenu:hotkey-changed', state.codes);
		}
	} catch {
	}
}

function codeVariants(code: string): string[] {
	const modifier = ['Alt', 'Control', 'Meta', 'Shift'].find(
		(prefix) => code === prefix || code === `${prefix}Left` || code === `${prefix}Right`,
	);
	if (!modifier) return [code];
	if (code === modifier) return [`${modifier}Left`, `${modifier}Right`];
	if (code.endsWith('Left')) return [code, `${code.slice(0, -4)}Right`];
	if (code.endsWith('Right')) return [code, `${code.slice(0, -5)}Left`];
	return [code];
}

export function hotkeyWatchCodes(): Set<string> {
	return new Set(hotkeyCodes().filter(Boolean).flatMap(codeVariants));
}

export function matchesHotkey(pressed: Set<string>): boolean {
	const slots = hotkeyCodes().filter(Boolean);
	if (slots.length === 0) return false;
	return slots.every((code) => codeVariants(code).some((variant) => pressed.has(variant)));
}
