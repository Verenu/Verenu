// Lightweight platform detection for UI labels. Uses the WebView user-agent so
// no extra Tauri plugin/dependency is required. The backend remains the source
// of truth for actual platform behavior.
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
// Some Android WebViews used by Tauri report a desktop-style UA. Their
// platform still identifies the Linux/ARM WebView. Verenu's supported desktop
// targets are Windows/macOS, so Linux + touch is an Android fallback without
// depending on a Tauri bootstrap global that may not exist at module load.
const isAndroidWebViewFallback =
	/Linux/i.test(ua) &&
	!/X11/i.test(ua);
const isTouchOnlyRuntime =
	!isMacUserAgent &&
	typeof navigator !== 'undefined' &&
	(navigator.maxTouchPoints > 0 ||
		(typeof window !== 'undefined' &&
			typeof window.matchMedia === 'function' &&
			window.matchMedia('(pointer: coarse)').matches));
// The desktop shell enforces a 900px minimum width. A phone WebView can
// therefore be identified safely from its CSS viewport even when its UA and
// native bridge are unavailable during the first cold render.
const isPhoneViewport =
	typeof window !== 'undefined' &&
	window.innerWidth <= 600;
const detectedAndroid =
	/Android/i.test(ua) ||
	/Android/i.test(uaDataPlatform) ||
	/VerenuAndroid/i.test(ua) ||
	isNativeAndroidRuntime ||
	isAndroidWebViewFallback ||
	isTouchOnlyRuntime ||
	isPhoneViewport;
export const isAndroid = detectedAndroid;
// A desktop-style Android UA may still match /Windows/; the explicit native
// marker above must win so all platform labels agree on the real target.
export const isWindows = isWindowsUserAgent && !isAndroid;
/** Phone/tablet/foldable shell: touch-first layout, bottom nav, safe-area insets. */
export const isMobile = isAndroid;
/** Coarse pointers need larger hit targets regardless of OS. */
export const isTouchDevice =
	typeof window !== 'undefined' &&
	typeof window.matchMedia !== 'undefined' &&
	window.matchMedia('(pointer: coarse)').matches;

/** Human label for a `KeyboardEvent.code`, OS-aware (⌘/⌃/⌥ + fn on macOS). */
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
		MetaLeft: 'Windows',
		MetaRight: 'Windows',
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
export const defaultHotkey: string[] = isMac ? ['AltLeft', 'Space'] : ['ControlLeft', 'MetaLeft'];

/** Platform label for the fixed copy-last-dictation shortcut. */
export const copyLastHotkey: string[] = isMac
	? ['AltLeft', 'MetaLeft', 'KeyC']
	: ['ControlLeft', 'AltLeft', 'KeyC'];
