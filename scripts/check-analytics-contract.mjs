import { readFileSync } from 'node:fs';
import { readdirSync } from 'node:fs';
import { resolve } from 'node:path';

const root = resolve(import.meta.dirname, '..');
const kotlinRoot = resolve(root, 'src-tauri/android/kotlin');
const boundary = readFileSync(resolve(kotlinRoot, 'com/verenu/app/VerenuAnalytics.kt'), 'utf8');
const desktopBoundary = readFileSync(resolve(root, 'src-tauri/src/analytics.rs'), 'utf8');
const desktopProduction = desktopBoundary.split('#[cfg(test)]')[0];

const files = [];
const walk = (dir) => {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = resolve(dir, entry.name);
    if (entry.isDirectory()) walk(path);
    else if (entry.name.endsWith('.kt')) files.push(path);
  }
};

// Keep this check intentionally simple and reviewable: application Kotlin may
// use VerenuAnalytics, but only the boundary may reference the SDK symbol.
walk(kotlinRoot);
for (const file of files) {
  const source = readFileSync(file, 'utf8');
  if (file !== resolve(kotlinRoot, 'com/verenu/app/VerenuAnalytics.kt') && /\bPostHog\b/.test(source)) {
    throw new Error(`direct PostHog reference outside analytics boundary: ${file}`);
  }
}

for (const forbidden of ['error', 'exception', 'transcript', 'clipboard', 'package', 'window', 'api_key', 'stack_trace']) {
  if (new RegExp(`['"]${forbidden}['"]\\s*:`, 'i').test(boundary)) {
    throw new Error(`sensitive telemetry property reached analytics boundary: ${forbidden}`);
  }
}

for (const required of ['analytics_schema_version', 'UUID.randomUUID()', '\\$geoip_disable', '\\$process_person_profile']) {
  if (!boundary.includes(required)) throw new Error(`Android analytics boundary is missing required privacy control: ${required}`);
}

for (const file of [resolve(root, 'src-tauri/src/lib.rs'), resolve(root, 'src-tauri/src/main.rs'), resolve(root, 'src-tauri/src/pipeline/mod.rs'), resolve(root, 'src-tauri/src/pipeline/session.rs')]) {
  if (/PostHog|["']\/capture\//i.test(readFileSync(file, 'utf8'))) {
    throw new Error(`direct transport reference outside desktop analytics boundary: ${file}`);
  }
}
for (const required of ['analytics_schema_version', 'Uuid::new_v4', 'analytics_install_id', 'create_new(true)', 'analytics_first_seen_version', '$geoip_disable', '$process_person_profile']) {
  if (!desktopProduction.includes(required)) throw new Error(`desktop analytics boundary is missing required privacy control: ${required}`);
}
for (const forbidden of ['MachineGuid', 'hardware UUID', 'Android ID', 'advertising ID', 'pairing UUID', 'hostname']) {
  if (desktopProduction.includes(forbidden)) throw new Error(`desktop identity references forbidden source: ${forbidden}`);
}
for (const required of ['getSharedPreferences(IDENTITY_PREFS', 'PostHog.identify(installId', 'reuseAnonymousId = true', 'firstSeenVersion', 'identity_schema_version']) {
  if (!boundary.includes(required)) throw new Error(`Android analytics identity is missing required continuity control: ${required}`);
}
for (const forbidden of ['ANDROID_ID', 'AdvertisingIdClient', 'Build.SERIAL', 'Settings.Secure', 'pairingUuid', 'deviceId']) {
  if (boundary.includes(forbidden)) throw new Error(`Android identity references forbidden source: ${forbidden}`);
}
for (const forbidden of ['transcript', 'clipboard_text', 'window_title', 'exception_message', 'error_text', 'stack_trace']) {
  if (new RegExp(`['"]${forbidden}['"]\\s*:`, 'i').test(desktopProduction)) {
    throw new Error(`sensitive value reference reached desktop analytics boundary: ${forbidden}`);
  }
}
console.log(`analytics contract ok: ${files.length} Kotlin files checked`);
