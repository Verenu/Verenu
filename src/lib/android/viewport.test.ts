import { describe, expect, it } from 'vitest';
import {
	shouldUseBottomNav,
	shouldUseMultiPane,
	snapshotForSize,
	trackViewport,
	widthClassForDp,
} from './viewport';

describe('widthClassForDp', () => {
	it('classifies compact phones and narrow outer foldable displays', () => {
		expect(widthClassForDp(360)).toBe('compact');
		expect(widthClassForDp(500)).toBe('compact');
		expect(widthClassForDp(599)).toBe('compact');
	});

	it('classifies medium widths (landscape phones, split-screen, small tablets)', () => {
		expect(widthClassForDp(600)).toBe('medium');
		expect(widthClassForDp(700)).toBe('medium');
		expect(widthClassForDp(839)).toBe('medium');
	});

	it('classifies expanded widths (unfolded Fold, tablets, freeform windows)', () => {
		expect(widthClassForDp(840)).toBe('expanded');
		expect(widthClassForDp(1280)).toBe('expanded');
	});

	it('degrades invalid input to compact instead of crashing layout', () => {
		expect(widthClassForDp(NaN)).toBe('compact');
		expect(widthClassForDp(-10)).toBe('compact');
	});
});

describe('snapshotForSize', () => {
	it('detects orientation from live dimensions', () => {
		expect(snapshotForSize(360, 800).orientation).toBe('portrait');
		expect(snapshotForSize(800, 360).orientation).toBe('landscape');
	});

	it('keeps non-expanded postures flat', () => {
		expect(snapshotForSize(400, 800, 'half-opened').foldPosture).toBe('flat');
		expect(snapshotForSize(900, 800, 'half-opened').foldPosture).toBe('half-opened');
	});
});

describe('layout decisions', () => {
	it('uses bottom nav only on compact', () => {
		expect(shouldUseBottomNav(snapshotForSize(360, 780))).toBe(true);
		expect(shouldUseBottomNav(snapshotForSize(700, 500))).toBe(false);
		expect(shouldUseBottomNav(snapshotForSize(1200, 800))).toBe(false);
	});

	it('uses multi-pane only on flat expanded windows', () => {
		expect(shouldUseMultiPane(snapshotForSize(1200, 800))).toBe(true);
		expect(shouldUseMultiPane(snapshotForSize(700, 500))).toBe(false);
		// Half-opened foldable: hinge area makes spanning panes unreadable.
		expect(shouldUseMultiPane(snapshotForSize(1200, 800, 'half-opened'))).toBe(false);
	});
});

describe('trackViewport', () => {
	it('emits immediately and on resize without restarting', () => {
		const seen: string[] = [];
		const listeners = new Map<string, () => void>();
		const events = {
			addEventListener: (event: string, cb: () => void) => listeners.set(event, cb),
			removeEventListener: (event: string) => void listeners.delete(event),
		};
		const removed: string[] = [];
		const removingEvents = {
			addEventListener: events.addEventListener,
			removeEventListener: (event: string) => {
				removed.push(event);
				listeners.delete(event);
			},
		};
		const frames = {
			requestAnimationFrame: (cb: () => void) => {
				cb();
				return 1;
			},
			cancelAnimationFrame: () => {},
		};

		let size = { width: 360, height: 780 };
		const stop = trackViewport(
			() => size,
			(snapshot) => seen.push(snapshot.widthClass),
			removingEvents,
			frames,
		);

		size = { width: 1200, height: 800 };
		listeners.get('resize')?.();
		stop();

		expect(seen).toEqual(['compact', 'expanded']);
		expect(removed).toEqual(['resize', 'orientationchange']);
	});
});
