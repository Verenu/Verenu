export type WidthClass = 'compact' | 'medium' | 'expanded';

export const WIDTH_CLASS_BREAKPOINTS = { medium: 600, expanded: 840 } as const;

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

export function shouldUseMultiPane(snapshot: ViewportSnapshot): boolean {
	if (snapshot.foldPosture === 'half-opened') return false;
	return snapshot.widthClass === 'expanded';
}

export function shouldUseBottomNav(snapshot: ViewportSnapshot): boolean {
	return snapshot.widthClass !== 'expanded';
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
