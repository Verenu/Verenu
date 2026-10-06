import { describe, expect, it } from 'vitest';
import { parseSetupProgress, resumeStep } from './setupProgress';

const providers = ['groq', 'openai', 'google', 'local'];
const steps = { apiKeyStep: 3, doneStep: 8 };

describe('parseSetupProgress', () => {
  it('keeps a valid step and provider', () => {
    expect(parseSetupProgress({ step: 4, provider: 'google' }, providers)).toEqual({ step: 4, provider: 'google' });
  });

  it('drops an unknown provider but keeps the step', () => {
    expect(parseSetupProgress({ step: 2, provider: 'mystery' }, providers)).toEqual({ step: 2 });
  });

  it.each([null, undefined, 3, 'x', {}, { step: -1 }, { step: 1.5 }, { step: '2' }])('rejects %j', (raw) => {
    expect(parseSetupProgress(raw, providers)).toBeNull();
  });
});

describe('resumeStep', () => {
  it('resumes where the user left off', () => {
    expect(resumeStep(5, { ...steps, keyReady: true })).toBe(5);
  });

  it('returns to the key step when the later steps would have no key', () => {
    expect(resumeStep(5, { ...steps, keyReady: false })).toBe(3);
  });

  it('lets someone who has not reached the key step stay before it', () => {
    expect(resumeStep(2, { ...steps, keyReady: false })).toBe(2);
    expect(resumeStep(3, { ...steps, keyReady: false })).toBe(3);
  });

  it('stays inside the wizard', () => {
    expect(resumeStep(40, { ...steps, keyReady: true })).toBe(8);
    expect(resumeStep(-2, { ...steps, keyReady: true })).toBe(0);
  });
});
