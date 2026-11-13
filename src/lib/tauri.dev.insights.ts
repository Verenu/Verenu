type DevPayload = ReturnType<typeof baseInsights>;

export function devInsightsPricing(): unknown {
  return { fetched_at: 0, rates: [] };
}

/**
 * Browser-preview insights. `scenario` (from localStorage `verenu:dev-insights-scenario`)
 * reshapes the payload so edge cases can be inspected without a real database.
 */
export function devInsights(
  days: number,
  contextId: number | null,
  contextIds: number[] = [],
  scenario: string | null = null,
): unknown {
  const payload = baseInsights(days, contextId, contextIds);
  if (scenario === 'empty') return emptyScenario(payload);
  if (scenario === 'single') return singleScenario(payload);
  if (scenario === 'extreme') return extremeScenario(payload);
  if (scenario === 'fresh') return freshScenario(payload);
  return payload;
}

function zeroDays(days: Array<{ day: string; words: number; transcriptions: number; speaking_ms: number }>) {
  return days.map((d) => ({ ...d, words: 0, transcriptions: 0, speaking_ms: 0 }));
}

function emptyScenario(p: DevPayload): DevPayload {
  return {
    ...p,
    totals: { ...p.totals, total_words: 0, total_transcriptions: 0, total_speaking_ms: 0, avg_words_per_transcription: 0, avg_wpm: 0, best_wpm: 0, words_in_range: 0, words_prev_range: 0 },
    streak: { current_days: 0, longest_days: 0, longest_started_on: null, longest_ended_on: null, longest_words: 0, active_days: 0 },
    daily: zeroDays(p.daily),
    streak_daily: zeroDays(p.streak_daily),
    history_started_on: null,
    hourly: new Array(24).fill(0),
    providers: [],
    cleanup: { ...p.cleanup, raw_words: 0, clean_words: 0, edits_applied: 0, vocabulary_replacements: 0, snippet_expansions: 0, activity_tracked_dictations: 0, text_dictations: 0 },
    words: { top: [], unique_words: 0, longest_word: null, avg_word_length: 0 },
    contexts: [],
  };
}

function singleScenario(p: DevPayload): DevPayload {
  const daily = zeroDays(p.daily);
  const today = daily[daily.length - 1];
  if (today) Object.assign(today, { words: 12, transcriptions: 1, speaking_ms: 6_000 });
  const streakDaily = zeroDays(p.streak_daily);
  Object.assign(streakDaily[streakDaily.length - 1], { words: 12, transcriptions: 1, speaking_ms: 6_000 });
  const hourly = new Array(24).fill(0);
  hourly[9] = 12;
  return {
    ...p,
    totals: { total_words: 12, total_transcriptions: 1, total_speaking_ms: 6_000, avg_words_per_transcription: 12, avg_wpm: 120, best_wpm: 120, words_in_range: 12, words_prev_range: 0 },
    streak: { current_days: 1, longest_days: 1, longest_started_on: today?.day ?? null, longest_ended_on: today?.day ?? null, longest_words: 12, active_days: 1 },
    daily,
    streak_daily: streakDaily,
    history_started_on: today?.day ?? null,
    hourly,
    providers: [{ model: 'whisper-large-v3-turbo', provider: 'groq', task: 'transcription', calls: 1, audio_ms: 6_000, input_chars: 0, output_chars: 0 }],
    cleanup: { raw_words: 13, clean_words: 12, edits_applied: 1, dictionary_fixes: 0, auto_learned_terms: 0, vocabulary_replacements: 0, snippet_expansions: 0, activity_incomplete: false, activity_tracked_dictations: 1, text_dictations: 1 },
    words: { top: [{ word: 'hello', count: 1 }], unique_words: 9, longest_word: 'dictation', avg_word_length: 4.2 },
    contexts: p.contexts.slice(0, 1).map((c) => ({ ...c, words: 12, transcriptions: 1 })),
  };
}

function freshScenario(p: DevPayload): DevPayload {
  return {
    ...p,
    totals: { ...p.totals, words_prev_range: 0, best_wpm: 0, avg_wpm: 0 },
    cleanup: { ...p.cleanup, activity_incomplete: true, activity_tracked_dictations: 0, text_dictations: 0 },
  };
}

function extremeScenario(p: DevPayload): DevPayload {
  const k = 4_000;
  const daily = p.daily.map((d) => ({ ...d, words: d.words * k, transcriptions: d.transcriptions * k, speaking_ms: d.speaking_ms * k }));
  const words = daily.reduce((sum, d) => sum + d.words, 0);
  return {
    ...p,
    totals: { ...p.totals, total_words: words * 9, total_transcriptions: p.totals.total_transcriptions * k, total_speaking_ms: p.totals.total_speaking_ms * k, avg_words_per_transcription: 9_999, avg_wpm: 412.6, best_wpm: 1_250, words_in_range: words, words_prev_range: Math.round(words / 40) },
    daily,
    streak_daily: p.streak_daily.map((d) => ({ ...d, words: d.words * k })),
    hourly: p.hourly.map((n) => n * k),
    providers: [
      ...p.providers.map((row) => ({ ...row, calls: row.calls * k, audio_ms: row.audio_ms * k, input_chars: row.input_chars * k, output_chars: row.output_chars * k })),
      { model: 'some-provider/an-extremely-long-model-identifier-that-keeps-going-v9000-preview-instruct', provider: 'openrouter', task: 'cleanup', calls: 3, audio_ms: 0, input_chars: 900, output_chars: 800 },
    ],
    cleanup: { ...p.cleanup, raw_words: words * 2, clean_words: words, edits_applied: 98_765_432, vocabulary_replacements: 1_234_567, snippet_expansions: 765_432 },
    words: {
      top: [
        { word: 'pneumonoultramicroscopicsilicovolcanoconiosis', count: 9_876_543 },
        { word: 'Donaudampfschifffahrtsgesellschaftskapitän', count: 4_321_000 },
        { word: 'ok', count: 12 },
      ],
      unique_words: 123_456_789,
      longest_word: 'a'.repeat(320),
      avg_word_length: 23.9,
    },
    contexts: p.contexts.map((c) => ({ ...c, words: c.words * k, transcriptions: c.transcriptions * k })),
  };
}

function baseInsights(days: number, contextId: number | null, contextIds: number[]) {
  const span = days > 0 ? days : 120;
  const scale = contextId === null ? 1 : 1 / (1 + (contextId % 5));
  const noise = (n: number) =>
    ((Math.sin((n + (contextId ?? 0) * 7) * 12.9898) * 43758.5453) % 1 + 1) % 1;

  const today = new Date();
  const daily = Array.from({ length: span }, (_, i) => {
    const date = new Date(today);
    date.setDate(today.getDate() - (span - 1 - i));
    const weekend = date.getDay() === 0 || date.getDay() === 6;
    const r = noise(i + 1);
    const idle = r < (weekend ? 0.45 : 0.12);
    const words = idle ? 0 : Math.round(400 + r * (weekend ? 1400 : 4200));
    const transcriptions = words === 0 ? 0 : Math.max(1, Math.round(words / 95));
    const pad = (n: number) => String(n).padStart(2, '0');
    return {
      day: `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`,
      words,
      transcriptions,
      speaking_ms: Math.round((words / 145) * 60_000),
    };
  });
  const streakStart = new Date(today);
  streakStart.setDate(today.getDate() - 364);
  const streakSpan = 365;
  const streakDaily = Array.from({ length: streakSpan }, (_, i) => {
    const date = new Date(today);
    date.setTime(streakStart.getTime());
    date.setDate(streakStart.getDate() + i);
    const weekend = date.getDay() === 0 || date.getDay() === 6;
    const r = noise(i + 101);
    const idle = r < (weekend ? 0.45 : 0.12);
    const words = idle ? 0 : Math.round(400 + r * (weekend ? 1400 : 4200));
    const transcriptions = words === 0 ? 0 : Math.max(1, Math.round(words / 95));
    const pad = (n: number) => String(n).padStart(2, '0');
    return {
      day: `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`,
      words,
      transcriptions,
      speaking_ms: Math.round((words / 145) * 60_000),
    };
  });

  for (const d of daily) {
    d.words = Math.round(d.words * scale);
    d.transcriptions = d.words === 0 ? 0 : Math.max(1, Math.round(d.transcriptions * scale));
    d.speaking_ms = Math.round(d.speaking_ms * scale);
  }

  const wordsInRange = daily.reduce((sum, d) => sum + d.words, 0);
  const transcriptions = daily.reduce((sum, d) => sum + d.transcriptions, 0);
  const speakingMs = daily.reduce((sum, d) => sum + d.speaking_ms, 0);

  let current = 0;
  for (let i = streakDaily.length - 1; i >= 0 && streakDaily[i].words > 0; i--) current++;
  let longest = 0;
  let run = 0;
  let runStart: string | null = null;
  let longestStartedOn: string | null = null;
  let longestEndedOn: string | null = null;
  for (const d of streakDaily) {
    if (d.words > 0) {
      if (run === 0) runStart = d.day;
      run += 1;
      if (run > longest) {
        longest = run;
        longestStartedOn = runStart;
        longestEndedOn = d.day;
      }
    } else {
      run = 0;
      runStart = null;
    }
  }

  const hourly = Array.from({ length: 24 }, (_, h) => {
    const bell = Math.exp(-((h - 14) ** 2) / 24) + 0.35 * Math.exp(-((h - 9) ** 2) / 8);
    return Math.round(bell * wordsInRange * 0.11);
  });

  return {
    context_id: contextId,
    range_days: days,
    generated_at: new Date().toISOString().slice(0, 19).replace('T', ' '),
    totals: {
      total_words: contextId === null ? wordsInRange + 218_400 : wordsInRange,
      total_transcriptions: transcriptions,
      total_speaking_ms: speakingMs,
      avg_words_per_transcription: transcriptions ? Math.round(wordsInRange / transcriptions) : 0,
      avg_wpm: 148,
      best_wpm: 197,
      words_in_range: wordsInRange,
      words_prev_range: Math.round(wordsInRange * 0.91),
    },
    streak: {
      current_days: current,
      longest_days: longest,
      longest_started_on: longestStartedOn,
      longest_ended_on: longestEndedOn,
      longest_words: Math.round(wordsInRange * 0.62),
      active_days: streakDaily.filter((d) => d.words > 0).length,
    },
    daily,
    streak_daily: streakDaily,
    history_started_on: (streakDaily[Math.max(0, streakDaily.length - 240)]?.day ?? null) as string | null,
    hourly,
    providers: [
      {
        model: 'whisper-large-v3-turbo',
        provider: 'groq',
        task: 'transcription',
        calls: transcriptions,
        audio_ms: speakingMs,
        input_chars: 0,
        output_chars: 0,
      },
      {
        model: 'qwen/qwen3.8-27b',
        provider: 'groq',
        task: 'cleanup',
        calls: Math.round(transcriptions * 0.86),
        audio_ms: 0,
        input_chars: wordsInRange * 6,
        output_chars: wordsInRange * 5,
      },
      {
        model: 'gemini-3.5-flash-lite',
        provider: 'google',
        task: 'cleanup',
        calls: Math.round(transcriptions * 0.14),
        audio_ms: 0,
        input_chars: Math.round(wordsInRange * 0.9),
        output_chars: Math.round(wordsInRange * 0.8),
      },
    ],
    cleanup: {
      raw_words: Math.round(wordsInRange * 1.08),
      clean_words: wordsInRange,
      edits_applied: Math.round(wordsInRange * 0.031),
      dictionary_fixes: Math.round(wordsInRange * 0.009),
      auto_learned_terms: 24,
      vocabulary_replacements: Math.round(wordsInRange * 0.009),
      snippet_expansions: 12,
      activity_incomplete: false,
      activity_tracked_dictations: transcriptions,
      text_dictations: transcriptions,
    },
    words: {
      top: [
        { word: 'transcription', count: 412 },
        { word: 'component', count: 388 },
        { word: 'settings', count: 341 },
        { word: 'basically', count: 297 },
        { word: 'pipeline', count: 264 },
        { word: 'window', count: 231 },
        { word: 'actually', count: 210 },
        { word: 'clipboard', count: 188 },
        { word: 'dictation', count: 165 },
        { word: 'backend', count: 142 },
        { word: 'shortcut', count: 121 },
        { word: 'accent', count: 104 },
      ],
      unique_words: 7_412,
      longest_word: 'internationalisation',
      avg_word_length: 4.7,
    } as { top: Array<{ word: string; count: number }>; unique_words: number; longest_word: string | null; avg_word_length: number },
    contexts: contextId === null ? splitByContext(wordsInRange, transcriptions, contextIds) : [],
  };
}

function splitByContext(words: number, transcriptions: number, ids: number[]) {
  const weights = [0.46, 0.27, 0.14, 0.08, 0.05];
  return ids.slice(0, weights.length).map((id, i) => {
    const share = weights[i] / weights.slice(0, Math.min(ids.length, weights.length)).reduce((a, b) => a + b, 0);
    return { context_id: id, words: Math.round(words * share * 0.94), transcriptions: Math.max(1, Math.round(transcriptions * share * 0.94)) };
  });
}
