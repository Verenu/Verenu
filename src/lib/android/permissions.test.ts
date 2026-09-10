import { describe, expect, it } from 'vitest';
import {
	isFunctional,
	missingRequired,
	parseGrant,
	recoveryCopy,
	type AndroidPermissionSnapshot,
} from './permissions';

const functional: AndroidPermissionSnapshot = {
	microphone: 'granted',
	accessibility_service: 'granted',
	battery_exemption: 'denied',
	notifications: 'not_asked',
};

describe('isFunctional', () => {
	it('requires mic and accessibility, tolerates battery/notification gaps', () => {
		expect(isFunctional(functional)).toBe(true);
		expect(
			isFunctional({ ...functional, microphone: 'denied' }),
		).toBe(false);
		expect(
			isFunctional({ ...functional, accessibility_service: 'not_asked' }),
		).toBe(false);
		// OEM battery managers and notification opt-outs never block dictation.
		expect(
			isFunctional({ ...functional, battery_exemption: 'granted', notifications: 'granted' }),
		).toBe(true);
	});
});

describe('missingRequired', () => {
	it('lists missing gates in onboarding order', () => {
		expect(
			missingRequired({
				microphone: 'denied',
				accessibility_service: 'denied',
				battery_exemption: 'granted',
				notifications: 'granted',
			}),
		).toEqual(['microphone', 'accessibility_service']);
		expect(missingRequired(functional)).toEqual([]);
	});
});

describe('recoveryCopy', () => {
	it('tells permanently-denied users to open Settings', () => {
		expect(recoveryCopy('microphone', 'permanently_denied')).toMatch(/settings/i);
		expect(recoveryCopy('accessibility_service', 'permanently_denied')).toMatch(
			/Accessibility/i,
		);
	});

	it('offers a direct next step for askable gates', () => {
		expect(recoveryCopy('microphone', 'denied')).toMatch(/microphone/i);
		expect(recoveryCopy('battery_exemption', 'not_asked')).toMatch(/battery/i);
	});
});

describe('parseGrant', () => {
	it('covers Kotlin snake_case and camelCase ids', () => {
		expect(parseGrant('granted')).toBe('granted');
		expect(parseGrant('denied')).toBe('denied');
		expect(parseGrant('permanently_denied')).toBe('permanently_denied');
		expect(parseGrant('permanentlyDenied')).toBe('permanently_denied');
		expect(parseGrant('unexpected')).toBe('not_asked');
	});
});
