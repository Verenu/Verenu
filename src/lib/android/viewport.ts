// Adaptive window-size classes for the Android shell (and any narrow
// desktop window). Mirrors the Rust `crate::android::width_class_for_dp`
// breakpoints exactly — both sides use 600/840dp so backend layout hints and
// the Svelte shell never disagree.
//
// Uses live window dimensions, never hard-coded device categories: rotation,
// folding/unfolding, split-screen, and freeform windows all reclassify
// without restart. Hinge/fold posture comes from the CSS `screen-fold`
// feature where the WebView supports it; otherwise the width class alone
// drives the layout (compact → single column + bottom nav, expanded →
// list-detail multi-pane).

export type WidthClass = 'compact' | 'medium' | 'expanded';

export const WIDTH_CLASS_BREAKPOINTS = { medium: 600, expanded: 840 } as const;

/** Pure: classify an available width in density-independent pixels. */
export function widthClassForDp(widthDp: number): WidthClass {
	if (!Number.isFinite(widthDp) || widthDp < 0) return 'compact';
	if (widthDp < WIDTH_CLASS_BREAKPOINTS.medium) return 'compact';
	if (widthDp < WIDTH_CLASS_BREAKPOINTS.expanded) return 'medium';
	return 'expanded';
}

/** Approximate dp from CSS pixels. Android WebViews report CSS px ≈ dp. */
export function cssPxToDp(cssPx: number): number {
	return cssPx;
}

export type ViewportSnapshot = {
	widthDp: number;
	heightDp: number;
	widthClass: WidthClass;
	orientation: 'portrait' | 'landscape';
	/** True while a foldable is half-opened (tabletop/book) where detectable. */
	foldPosture: 'flat' | 'half-opened' | 'unknown';
};

function readOrientation(width: number, height: number): ViewportSnapshot['orientation'] {
	return height >= width ? 'portrait' : 'landscape';
}

/**
 * Pure: build a snapshot from raw dimensions. `foldPosture` is best-effort —
 * callers pass `'half-opened'` when the Jetpack WindowManager posture API
 * (bridged via `android_get_window_metrics`) reports it, else `'unknown'`.
 */
export function snapshotForSize(
	widthDp: number,
	heightDp: number,
	foldPosture: ViewportSnapshot['foldPosture'] = 'unknown',
): ViewportSnapshot {
	const widthClass = widthClassForDp(widthDp);
	return {
		widthDp,
		heightDp,
		widthClass,
		orientation: readOrientation(widthDp, heightDp),
		foldPosture: widthClass === 'expanded' ? foldPosture : 'flat',
	};
}

/** Whether the main shell should use the multi-pane (list-detail) layout. */
export function shouldUseMultiPane(snapshot: ViewportSnapshot): boolean {
	// Half-opened foldables keep single-column even when wide: the hinge
	// area makes a spanning detail pane unreadable.
	if (snapshot.foldPosture === 'half-opened') return false;
	return snapshot.widthClass === 'expanded';
}

/** Whether primary navigation collapses to the bottom bar. */
export function shouldUseBottomNav(snapshot: ViewportSnapshot): boolean {
	return snapshot.widthClass === 'compact';
}

export type ViewportListener = (snapshot: ViewportSnapshot) => void;

export type ViewportEventTarget = {
	addEventListener: (event: string, handler: () => void) => void;
	removeEventListener: (event: string, handler: () => void) => void;
};

export type AnimationFrameScope = {
	requestAnimationFrame: (cb: () => void) => number;
	cancelAnimationFrame: (id: number) => void;
};

function defaultEventTarget(): ViewportEventTarget | null {
	if (typeof window !== 'undefined' && typeof window.addEventListener === 'function') {
		return window;
	}
	return null;
}

function defaultFrameScope(): AnimationFrameScope | null {
	if (typeof window !== 'undefined' && typeof window.requestAnimationFrame === 'function') {
		return window;
	}
	return null;
}

/**
 * Live tracker for `App.svelte`. Subscribes to resize + orientation changes
 * and emits a fresh snapshot (debounced to animation frames so rotation and
 * live fold/unfold don't thrash the layout). Returns an unsubscribe fn.
 * Pure enough to unit-test via `snapshotForSize`; this wrapper only owns the
 * DOM subscription. Pass explicit `events`/`frames` doubles in tests — the
 * default is the global window, and a null window (SSR/tests) emits once.
 */
export function trackViewport(
	getSize: () => { width: number; height: number },
	notify: ViewportListener,
	events: ViewportEventTarget | null = defaultEventTarget(),
	frames: AnimationFrameScope | null = defaultFrameScope(),
): () => void {
	let raf = 0;
	let disposed = false;
	const emit = () => {
		if (disposed) return;
		raf = 0;
		const { width, height } = getSize();
		notify(snapshotForSize(cssPxToDp(width), cssPxToDp(height)));
	};
	const schedule = () => {
		if (disposed) return;
		if (raf !== 0 || !frames) return;
		raf = frames.requestAnimationFrame(emit);
	};
	emit();
	events?.addEventListener('resize', schedule);
	events?.addEventListener('orientationchange', schedule);
	return () => {
		disposed = true;
		if (raf !== 0) frames?.cancelAnimationFrame(raf);
		events?.removeEventListener('resize', schedule);
		events?.removeEventListener('orientationchange', schedule);
	};
}
