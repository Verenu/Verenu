import { describe, expect, it } from 'vitest';
import { CUSTOM_PROVIDER_PRESETS, POPULAR_PRESET_IDS, displayHost, monogram, presetForUrl } from './customProviderPresets';

describe('custom provider presets', () => {
  it('has unique ids and names', () => {
    expect(new Set(CUSTOM_PROVIDER_PRESETS.map(p => p.id)).size).toBe(CUSTOM_PROVIDER_PRESETS.length);
    expect(new Set(CUSTOM_PROVIDER_PRESETS.map(p => p.name)).size).toBe(CUSTOM_PROVIDER_PRESETS.length);
  });

  it('only uses plain http for local addresses and https otherwise', () => {
    const urls = CUSTOM_PROVIDER_PRESETS.flatMap(p => [p.base_url, p.alt?.base_url ?? '']).filter(Boolean);
    for (const base_url of urls) {
      const url = new URL(base_url);
      expect(url.search + url.hash).toBe('');
      if (url.protocol === 'http:') expect(['localhost', '127.0.0.1']).toContain(url.hostname);
      else expect(url.protocol).toBe('https:');
      expect(base_url.endsWith('/')).toBe(false);
    }
  });

  it('respects protocol capabilities and name limits', () => {
    for (const p of CUSTOM_PROVIDER_PRESETS) {
      expect(p.name.length).toBeLessThanOrEqual(40);
      expect(p.supports_transcription || p.supports_cleanup).toBe(true);
      if (p.protocol === 'anthropic') expect(p.supports_transcription).toBe(false);
      if (p.alt) expect(p.alt.protocol).not.toBe(p.protocol);
      if (!p.supports_transcription) expect(p.transcription_models).toEqual([]);
      if (!p.supports_cleanup) expect(p.cleanup_models).toEqual([]);
    }
  });

  it('lists each vendor once and resolves popular ids', () => {
    for (const id of POPULAR_PRESET_IDS) expect(CUSTOM_PROVIDER_PRESETS.some(p => p.id === id)).toBe(true);
    expect(CUSTOM_PROVIDER_PRESETS.filter(p => /deepseek|moonshot|z\.ai/i.test(p.name))).toHaveLength(3);
    expect(presetForUrl('https://api.deepseek.com/anthropic', 'anthropic')?.id).toBe('deepseek');
  });

  it('builds readable marks and hosts', () => {
    expect(monogram('DeepSeek')).toBe('DS');
    expect(monogram('Together AI')).toBe('TA');
    expect(monogram('')).toBe('?');
    expect(displayHost('http://localhost:11434/v1')).toBe('localhost:11434');
    expect(presetForUrl('https://api.mistral.ai/v1', 'openai')?.id).toBe('mistral');
    expect(presetForUrl('https://example.com/v1', 'openai')).toBeUndefined();
  });
});
