/*
 * Browser-dev stand-in for `get_insights`. Deterministic (seeded off the day
 * index, no Math.random) so the page doesn't flicker between renders and the
 * smoke tests see stable numbers.
 */
export function devInsights(days: number, contextId: number | null): unknown {
  const span = days > 0 ? days : 120;
  // Deterministic per-context scaling: enough for the filter to visibly change
  // the page in browser dev mode without inventing a second fake dataset.
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
    history_started_on: streakDaily[Math.max(0, streakDaily.length - 240)]?.day ?? null,
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
    },
  };
}
