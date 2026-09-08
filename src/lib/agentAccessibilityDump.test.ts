import { describe, expect, it } from 'vitest';
import {
  MAX_DUMP_CHARS,
  buildAgentDump,
  compactDumpTitle,
  describeDumpElement,
  pushDumpEvent,
  redactSettings,
  redactValue,
  shouldRedactSettingKey,
} from './agentAccessibilityDump';

const baseSnapshot = {
  windowKind: 'main' as const,
  version: '0.18.1',
  extras: { page: 'home', pillState: 'idle', isOnline: true, settingsOpen: false },
  settings: {
    transcription_provider: 'groq',
    transcription_model: 'whisper-large-v3-turbo',
    cleanup_provider: 'groq',
    cleanup_model: 'qwen/qwen3.8-27b',
    microphone_device: 'Yeti Nano',
    clipboard_phrase: 'do not leak this',
  },
  errors: [] as string[],
  inventory: [] as string[],
  build: {
    commit: 'a83f92c',
    branch: 'gitbutler/workspace',
    dirty: true,
    timestamp: '2026-09-07T22:51:03Z',
  },
};

describe('agent accessibility dump', () => {
  it('redacts secrets and clipboard/prompt bodies, keeps ordinary settings', () => {
    expect(shouldRedactSettingKey('clipboard_phrase')).toBe(true);
    expect(shouldRedactSettingKey('cleanup_prompt_override')).toBe(true);
    expect(shouldRedactSettingKey('api_key_groq')).toBe(true);
    expect(shouldRedactSettingKey('transcription_provider')).toBe(false);
    expect(redactValue('paste clipboard here')).toBe('[redacted len=20]');
    expect(
      redactSettings({
        transcription_provider: 'groq',
        clipboard_phrase: 'secret phrase',
        api_key_openai: 'sk-test',
      }),
    ).toEqual({
      transcription_provider: 'groq',
      clipboard_phrase: '[redacted len=13]',
      api_key_openai: '[redacted len=7]',
    });
  });

  it('puts build identity, pipeline, and providers before bulky data', () => {
    const text = buildAgentDump({
      ...baseSnapshot,
      extras: {
        ...baseSnapshot.extras,
        page: 'settings',
        settingsSection: 'audio',
        contexts: {
          count: 6,
          selectedId: 12,
          names: Array.from({ length: 40 }, (_, index) => ({
            id: index,
            name: `context-${index}`,
            instructions: 'x'.repeat(200),
          })),
        },
      },
      errors: ['auth-401: key rejected'],
      recent: [{ at: Date.parse('2026-09-07T22:54:12Z'), name: 'pill-state', detail: 'recording' }],
    });

    expect(text).toContain('VERENU_AX_DUMP');
    expect(text.indexOf('build.commit=a83f92c')).toBeGreaterThan(-1);
    expect(text.indexOf('build.commit=')).toBeLessThan(text.indexOf('recent:'));
    expect(text.indexOf('pipeline=idle')).toBeLessThan(text.indexOf('recent:'));
    expect(text).toContain('last_error=auth-401: key rejected');
    expect(text).toContain('providers.stt=groq/whisper-large-v3-turbo');
    expect(text).toContain('22:54:12 pill-state recording');
    expect(text).toContain('transcription_provider=groq');
    expect(text).not.toContain('clipboard_phrase');
    expect(text).not.toContain('do not leak this');
    expect(text).not.toContain('context-12');
    expect(text).not.toContain('"windowKind"');
  });

  it('caps dump size so the accessibility tree stays queryable', () => {
    const inventory = Array.from({ length: 4000 }, (_, index) => `row-${index}-${'x'.repeat(20)}`);
    const text = buildAgentDump({
      ...baseSnapshot,
      windowKind: 'pill',
      inventory,
    });
    expect(text.length).toBeLessThanOrEqual(MAX_DUMP_CHARS + 80);
  });

  it('names the window title with version, commit, and page', () => {
    const title = compactDumpTitle(baseSnapshot);
    expect(title).toContain('Verenu DUMP');
    expect(title).toContain('v0.18.1');
    expect(title).toContain('a83f92c');
    expect(title).toContain('home');
  });

  it('describes a control with stable ids instead of hashed svelte classes', () => {
    const nav = {
      tagName: 'BUTTON',
      className: 'nav-item svelte-6dohdz active',
      getAttribute: (name: string) => {
        if (name === 'data-debug-id') return 'nav.home';
        return null;
      },
    } as unknown as Element;
    expect(describeDumpElement(nav)).toBe('id=nav.home component=nav-item state=active');
    expect(describeDumpElement(nav)).not.toContain('svelte-');

    const toggle = {
      tagName: 'BUTTON',
      className: 'toggle on',
      getAttribute: (name: string) => {
        if (name === 'role') return 'switch';
        if (name === 'aria-checked') return 'true';
        if (name === 'aria-label') return 'Ruin accessibility';
        return null;
      },
      closest: (selector: string) => {
        if (selector !== '[data-setting-target]') return null;
        return {
          getAttribute: () => 'developer-ruin-accessibility',
        };
      },
    } as unknown as Element;
    expect(describeDumpElement(toggle)).toContain('setting=developer-ruin-accessibility');
    expect(describeDumpElement(toggle)).toContain('value=true');
    expect(describeDumpElement(toggle)).toContain('component=toggle');
  });

  it('records recent events with deltas', () => {
    const first = pushDumpEvent('pill-state', 'recording');
    expect(first[first.length - 1]?.name).toBe('pill-state');
    const second = pushDumpEvent('pill-stage', 'transcribing');
    expect(second[second.length - 1]?.detail).toMatch(/transcribing \+\d+ms/);
  });
});
