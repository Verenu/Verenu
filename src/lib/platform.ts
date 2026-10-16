const ua = typeof navigator !== 'undefined' ? navigator.userAgent : '';
const uaDataPlatform =
	typeof navigator !== 'undefined'
		? (navigator as Navigator & { userAgentData?: { platform?: string } }).userAgentData
				?.platform ?? ''
		: '';

const isMacUserAgent = /Mac/i.test(ua);
const isWindowsUserAgent = /Win/i.test(ua);
const isNativeAndroidRuntime =
	typeof globalThis !== 'undefined' &&
	(globalThis as typeof globalThis & { __VERENU_ANDROID__?: boolean }).__VERENU_ANDROID__ === true;
export const isMac = isMacUserAgent;
export const isLinux = /Linux/i.test(ua) || /Linux/i.test(uaDataPlatform);
const isTauriWebView =
	typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
const isTouchPhone =
	!isMacUserAgent &&
	typeof navigator !== 'undefined' &&
	typeof window !== 'undefined' &&
	Math.min(window.innerWidth, window.innerHeight) <= 600 &&
	(navigator.maxTouchPoints > 0 ||
		(typeof window.matchMedia === 'function' &&
			window.matchMedia('(pointer: coarse)').matches));
const explicitAndroid =
	/Android/i.test(ua) ||
	/Android/i.test(uaDataPlatform) ||
	/VerenuAndroid/i.test(ua) ||
	isNativeAndroidRuntime;
export const isAndroid = explicitAndroid || (!isTauriWebView && isTouchPhone);
export const isLinuxDesktop = isLinux && !isAndroid;
export const isWindows = isWindowsUserAgent && !isAndroid;
export const isMobile = isAndroid;
export const isTouchDevice =
	typeof window !== 'undefined' &&
	typeof window.matchMedia !== 'undefined' &&
	window.matchMedia('(pointer: coarse)').matches;

export function formatKeyLabel(code: string): string {
	if (isMac) {
		const mac: Record<string, string> = {
			MetaLeft: '⌘',
			MetaRight: '⌘',
			ControlLeft: '⌃',
			ControlRight: '⌃',
			AltLeft: '⌥',
			AltRight: '⌥',
			ShiftLeft: '⇧',
			ShiftRight: '⇧',
			Fn: 'fn',
			CapsLock: '⇪',
			Space: 'Space'
		};
		if (mac[code]) return mac[code];
	}
	const generic: Record<string, string> = {
		ControlLeft: 'Ctrl',
		ControlRight: 'Ctrl',
		MetaLeft: isLinux ? 'Super' : 'Windows',
		MetaRight: isLinux ? 'Super' : 'Windows',
		AltLeft: 'Alt',
		AltRight: 'Alt',
		ShiftLeft: 'Shift',
		ShiftRight: 'Shift',
		Fn: 'Fn',
		Space: 'Space'
	};
	return (
		generic[code] ??
		code.replace('Left', '').replace('Right', '').replace('Key', '').replace('Digit', '')
	);
}

/**
 * The default dictation hotkey codes for the current platform.
 *
 * macOS uses Carbon `RegisterEventHotKey` (no Input Monitoring permission), which
 * can't bind a modifier-only chord — so the default is ⌥ Option + Space (two
 * adjacent bottom-row keys, no Fn/Spotlight conflict). Windows keeps its chord.
 */
export const defaultHotkey: string[] = isMac
	? ['AltLeft', 'Space']
	: ['ControlLeft', 'MetaLeft'];

export const copyLastHotkey: string[] = isMac
	? ['AltLeft', 'MetaLeft', 'KeyC']
	: ['ControlLeft', 'AltLeft', 'KeyC'];
