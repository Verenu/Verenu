// Android permission onboarding model. Mirrors the Rust
// `crate::android::{AndroidPermission, permission_rationale,
// permission_is_required}` contract so the wizard, the Settings recovery
// card, and the backend gate all agree on what "ready" means.
//
// Side-effectful parts (invoke/listen, opening system settings) stay in the
// Svelte step component; everything here is pure and unit-tested.

export type AndroidPermissionId =
	| 'microphone'
	| 'accessibility_service'
	| 'battery_exemption'
	| 'notifications';

export type PermissionGrant = 'granted' | 'denied' | 'not_asked' | 'permanently_denied';

export type AndroidPermissionSnapshot = {
	microphone: PermissionGrant;
	accessibility_service: PermissionGrant;
	battery_exemption: PermissionGrant;
	notifications: PermissionGrant;
};

export const ANDROID_PERMISSION_ORDER: AndroidPermissionId[] = [
	'microphone',
	'accessibility_service',
	'battery_exemption',
	'notifications',
];

/** Permissions without which dictation cannot function. */
export const REQUIRED_ANDROID_PERMISSIONS: AndroidPermissionId[] = [
	'microphone',
	'accessibility_service',
];

/** Fallback rationale shown if the backend rationale command is unreachable. */
export const FALLBACK_RATIONALE: Record<AndroidPermissionId, string> = {
	microphone:
		'Verenu needs microphone access to record your dictation. Audio is transcribed and then discarded.',
	accessibility_service:
		'Verenu uses an Accessibility Service to show the dictation pill above the keyboard and insert text into the focused field. It never runs when the keyboard is closed.',
	battery_exemption:
		'Some manufacturers kill background audio aggressively. Exempting Verenu keeps recordings from being cut off mid-sentence.',
	notifications:
		'Verenu posts a status notification while recording so you can see — and stop — a dictation from anywhere.',
};

/** Where each recovery action deep-links. Opened by the step component. */
export const PERMISSION_SETTINGS_TARGET: Record<AndroidPermissionId, string> = {
	microphone: 'app-details',
	accessibility_service: 'accessibility-settings',
	battery_exemption: 'battery-optimization-settings',
	notifications: 'app-notification-settings',
};

/** Dictation can function: mic + accessibility granted. */
export function isFunctional(snapshot: AndroidPermissionSnapshot): boolean {
	return (
		snapshot.microphone === 'granted' && snapshot.accessibility_service === 'granted'
	);
}

/** Required gates still missing, in onboarding order. */
export function missingRequired(snapshot: AndroidPermissionSnapshot): AndroidPermissionId[] {
	const missing: AndroidPermissionId[] = [];
	if (snapshot.microphone !== 'granted') missing.push('microphone');
	if (snapshot.accessibility_service !== 'granted') missing.push('accessibility_service');
	return missing;
}

/**
 * Recovery copy for a denied gate. `permanently_denied` means the system will
 * no longer show the prompt — the user must flip it in Settings, so say so
 * explicitly instead of offering a dead "ask again" button.
 */
export function recoveryCopy(
	permission: AndroidPermissionId,
	grant: PermissionGrant,
): string {
	if (grant === 'permanently_denied') {
		switch (permission) {
			case 'microphone':
				return 'Microphone access was blocked. Open the app settings and allow the microphone, then come back.';
			case 'accessibility_service':
				return 'The Verenu accessibility service is off. Open Accessibility settings, enable Verenu, then come back.';
			case 'battery_exemption':
				return 'Battery optimization is still restricting Verenu. Allow unrestricted battery use for reliable recordings.';
			case 'notifications':
				return 'Notifications are off. Enable them so the recording indicator can show while you dictate.';
		}
	}
	switch (permission) {
		case 'microphone':
			return 'Allow microphone access to start dictating.';
		case 'accessibility_service':
			return 'Enable the Verenu accessibility service to show the pill above your keyboard.';
		case 'battery_exemption':
			return 'Allow unrestricted battery use so long recordings are never cut off.';
		case 'notifications':
			return 'Allow notifications so the recording indicator can show.';
	}
}

/** Parse a grant id from Kotlin (snake_case or camelCase). */
export function parseGrant(id: string): PermissionGrant {
	switch (id) {
		case 'granted':
			return 'granted';
		case 'denied':
			return 'denied';
		case 'permanently_denied':
		case 'permanentlyDenied':
			return 'permanently_denied';
		default:
			return 'not_asked';
	}
}
