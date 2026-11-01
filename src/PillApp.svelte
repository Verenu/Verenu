<script lang="ts">
  import { classifyIpcError } from './lib/errors';
  import { onMount, tick } from 'svelte';
  import { createListenerScope } from './lib/listenerScope';
  import { createTimers } from './lib/timers';
  import { createPillSnapshotReceiver } from './lib/pillSnapshot';
  import { fly } from 'svelte/transition';
  import { BARS, createPillVisualizer } from './lib/pillVisualizer';
  import { pillBarHeight, pillBarSizing } from './lib/pillSizing';
  import AgentAccessibilityDump from './lib/components/AgentAccessibilityDump.svelte';
  import { isAndroid, isLinux } from './lib/platform';

  type PillState = 'idle' | 'recording' | 'processing' | 'loading_local_model' | 'handsfree' | 'error' | 'cancelled' | 'interrupted' | 'paste_failed' | 'copied' | 'clipboard_warning';
  const timers = createTimers<'errorTimer' | 'hfTimer' | 'cancelBtnTimer' | 'cancelDismissTimer' | 'copyBtnTimer' | 'pasteFailedDismissTimer' | 'copiedPillTimer' | 'copiedTimer' | 'dyingTimer' | 'audioGraceTimer' | 'audioZeroTimer' | 'audioStatusCooldownTimer' | 'stageTimer' | 'settleTimer'>();
  const terminalStates = new Set<PillState>(['error', 'cancelled', 'interrupted', 'paste_failed', 'copied', 'clipboard_warning']);
  const isTerminalState = (s: PillState) => terminalStates.has(s);
  const isCancelLike = (s: PillState) => s === 'cancelled' || s === 'interrupted';
  let state: PillState = 'idle';
  let errorMsg = '';
  $: displayError = classifyIpcError(errorMsg).message;
  let errOpen = false;
  let errWidth = 0;
  let errHeight = 34;
  let errLines = 1;
  let errScroll = false;
  let errTextEl: HTMLSpanElement | null = null;
  let errSizerEl: HTMLSpanElement | null = null;
  let showHfButtons = false;
  let cancelOpen = false;
  let showCancelBtn = false;
  let showCopyBtn = false;
  let copied = false;
  let prevState: PillState = 'idle';
  let dying = false;

  async function setPillInteractive(interactive: boolean) {
    const update = ++inputUpdate;
    const expectedState = state;
    inputChain = inputChain.then(async () => {
      await tick();
      if (update !== inputUpdate || state !== expectedState) return;
      const hasControls = !!clusterEl?.querySelector('.pill button');
      const enabled = interactive && hasControls && !dying;
      if (enabled) await reportHitRect(true);
      if (update !== inputUpdate || state !== expectedState) return;
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('set_pill_interactive', { interactive: enabled, expectedState });
    }).catch(() => {});
    await inputChain;
  }
  let inputUpdate = 0;
  let inputChain: Promise<void> = Promise.resolve();

  let contextLabel: string | null = null;

  type AudioStatus = 'starting' | 'healthy' | 'weak' | 'unheard' | 'muted';
  const AUDIO_START_GRACE_MS = 800;
  const AUDIO_ZERO_DEBOUNCE_MS = 600;
  const AUDIO_STATUS_STABLE_MS = 900;
  const AUDIO_ZERO_RMS = 0.00001;
  const AUDIO_MIN_SIGNAL_RMS = 0.0008;
  const AUDIO_NOISE_SEED_RMS = 0.0002;
  const AUDIO_NOISE_MULTIPLIER = 2.5;

  let audioStatus: AudioStatus = 'starting';
  let speechDetected = false;
  let audioIconEverShown = false;
  let rawAudioLevel = 0;
  let zeroInputConfirmed = false;
  let audioNoiseFloor = AUDIO_NOISE_SEED_RMS;
  let audioSessionStartedAt = 0;
  let audioStatusCandidate: AudioStatus | null = null;
  let audioStatusCandidateAt = 0;

  const isAudioState = (value: PillState) => value === 'recording' || value === 'handsfree';

  const reducedMotion = () =>
    typeof window !== 'undefined' && window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  const rollEase = (t: number) => 1 - Math.pow(1 - t, 3);

  function clearAudioStatusTimers() {
    timers.clear('audioGraceTimer', 'audioZeroTimer', 'audioStatusCooldownTimer');
  }

  function resetAudioStatus() {
    clearAudioStatusTimers();
    audioStatus = 'starting';
    speechDetected = false;
    rawAudioLevel = 0;
    zeroInputConfirmed = false;
    audioNoiseFloor = AUDIO_NOISE_SEED_RMS;
    audioSessionStartedAt = performance.now();
    audioStatusCandidate = null;
    timers.set('audioGraceTimer', () => {
      refreshAudioStatus();
    }, AUDIO_START_GRACE_MS);
  }

  function clearAudioStatus() {
    clearAudioStatusTimers();
    audioStatus = 'starting';
    speechDetected = false;
    rawAudioLevel = 0;
    zeroInputConfirmed = false;
    audioNoiseFloor = AUDIO_NOISE_SEED_RMS;
    audioSessionStartedAt = 0;
    audioStatusCandidate = null;
  }

  function updateNoiseFloor(level: number) {
    const followRate = level < audioNoiseFloor ? 0.25 : 0.03;
    audioNoiseFloor += (level - audioNoiseFloor) * followRate;
  }

  function meaningfulAudioLevel(level: number) {
    return level >= Math.max(AUDIO_MIN_SIGNAL_RMS, audioNoiseFloor * AUDIO_NOISE_MULTIPLIER);
  }

  function desiredAudioStatus(now = performance.now()): AudioStatus {
    if (!speechDetected && audioSessionStartedAt > 0 && now - audioSessionStartedAt < AUDIO_START_GRACE_MS) {
      return 'starting';
    }
    if (zeroInputConfirmed) return 'muted';
    if (speechDetected) return 'healthy';
    return meaningfulAudioLevel(rawAudioLevel) ? 'weak' : 'unheard';
  }

  function commitAudioStatus(next: AudioStatus) {
    audioStatusCandidate = null;
    timers.clear('audioStatusCooldownTimer');
    if (audioStatus === next) return;
    audioStatus = next;
  }

  function requestAudioStatus(next: AudioStatus, immediate = false) {
    if (!isAudioState(state) || next === audioStatus) {
      audioStatusCandidate = null;
      timers.clear('audioStatusCooldownTimer');
      return;
    }

    if (immediate) {
      commitAudioStatus(next);
      return;
    }

    const now = performance.now();
    if (audioStatusCandidate !== next) {
      audioStatusCandidate = next;
      audioStatusCandidateAt = now;
    }
    const elapsed = now - audioStatusCandidateAt;
    if (elapsed >= AUDIO_STATUS_STABLE_MS) {
      commitAudioStatus(next);
      return;
    }

    if (timers.has('audioStatusCooldownTimer')) return;
    timers.set('audioStatusCooldownTimer', () => {
      refreshAudioStatus();
    }, AUDIO_STATUS_STABLE_MS - elapsed);
  }

  function refreshAudioStatus(immediate = false) {
    if (!isAudioState(state)) return;
    requestAudioStatus(desiredAudioStatus(), immediate);
  }

  function onRawAudioLevel(level: number) {
    if (!isAudioState(state)) return;
    const nextLevel = Number.isFinite(level) ? Math.max(0, level) : 0;
    rawAudioLevel = nextLevel;

    if (nextLevel <= AUDIO_ZERO_RMS) {
      if (!timers.has('audioZeroTimer')) {
        timers.set('audioZeroTimer', () => {
          if (isAudioState(state) && rawAudioLevel <= AUDIO_ZERO_RMS) {
            zeroInputConfirmed = true;
            refreshAudioStatus(true);
          }
        }, AUDIO_ZERO_DEBOUNCE_MS);
      }
      return;
    }

    timers.clear('audioZeroTimer');
    zeroInputConfirmed = false;
    if (!speechDetected) updateNoiseFloor(nextLevel);

    refreshAudioStatus(speechDetected && audioStatus === 'muted');
  }

  function onSpeechDetected() {
    if (!isAudioState(state)) return;
    timers.clear('audioZeroTimer');
    zeroInputConfirmed = false;
    speechDetected = true;
    refreshAudioStatus(true);
  }

  function audioStatusDescription(status: AudioStatus) {
    switch (status) {
      case 'healthy': return 'Speech detected';
      case 'weak': return 'Audio is present, but speech detection is marginal';
      case 'unheard': return 'Speech has not been detected';
      case 'muted': return 'No microphone input';
      default: return 'Listening for speech';
    }
  }

  $: recordingAudioActive = isAudioState(state);
  $: if (recordingAudioActive) audioIconEverShown = true;
  $: pillContextTitle = recordingAudioActive
    ? `${contextLabel ? `${contextLabel}. ` : ''}${audioStatusDescription(audioStatus)}`
    : contextLabel ?? '';

  const STAGE_MIN_MS = 400;
  const STAGE_ROWS = ['Transcribing…', 'Cleaning…', 'Pasting…'] as const;
  const STAGE_INDEX: Record<string, number> = {
    transcribing: 0,
    cleaning: 1,
    pasting: 2,
  };
  const STAGE_ROW_H = 16;
  let stageIndex = -1;
  let pendingStageIndex: number | null = null;
  let stageShownAt = 0;

  let seenStages: number[] = [];

  function commitStage(idx: number) {
    if (!seenStages.includes(idx)) {
      seenStages = [...seenStages, idx].sort((a, b) => a - b);
    }
    stageIndex = idx;
    stageShownAt = performance.now();
  }

  function onPillStage(stageName: string) {
    const idx = STAGE_INDEX[stageName];
    if (
      idx === undefined ||
      (state !== 'processing' && state !== 'loading_local_model') ||
      stageIndex === idx
    ) return;
    if (stageIndex === -1 || performance.now() - stageShownAt >= STAGE_MIN_MS) {
      commitStage(idx);
      pendingStageIndex = null;
      return;
    }
    pendingStageIndex = idx;
    if (timers.has('stageTimer')) return;
    const remaining = STAGE_MIN_MS - (performance.now() - stageShownAt);
    timers.set('stageTimer', () => {
      if (pendingStageIndex !== null) {
        commitStage(pendingStageIndex);
        pendingStageIndex = null;
      }
    }, remaining);
  }

  function clearStage() {
    timers.clear('stageTimer');
    pendingStageIndex = null;
    stageIndex = -1;
    seenStages = [];
  }

  let stageWidthByLabel: Record<string, number> = {};
  const STAGE_FALLBACK_W = 72;
  const STAGE_WIDTH_SAFETY_PX = 3;

  function readStageWidths(node: HTMLElement) {
    const next = { ...stageWidthByLabel };
    let changed = false;
    node.querySelectorAll<HTMLElement>('[data-stage-label]').forEach((row) => {
      const label = row.dataset.stageLabel;
      const w = Math.ceil(row.getBoundingClientRect().width) + STAGE_WIDTH_SAFETY_PX;
      if (label && w > 0 && next[label] !== w) {
        next[label] = w;
        changed = true;
      }
    });
    if (changed) stageWidthByLabel = next;
  }

  function measureStageRows(node: HTMLElement) {
    readStageWidths(node);
    document.fonts?.ready
      .then(() => {
        if (node.isConnected) readStageWidths(node);
      })
      .catch(() => {});
    const ro =
      typeof ResizeObserver !== 'undefined'
        ? new ResizeObserver(() => readStageWidths(node))
        : null;
    ro?.observe(node);
    return {
      destroy() {
        ro?.disconnect();
      },
    };
  }
  $: stageMaxW = Object.keys(stageWidthByLabel).length
    ? Math.max(...Object.values(stageWidthByLabel))
    : STAGE_FALLBACK_W;
  $: stageW = stageWidthByLabel[STAGE_ROWS[stageIndex] ?? ''] ?? stageMaxW;

  $: activeStageRows = seenStages.map((i) => STAGE_ROWS[i]);
  $: rollPos = Math.max(0, seenStages.indexOf(stageIndex));

  const PILL_PAD_W = 20;
  const PILL_PAD_H = 20;
  const MIN_PILL_WINDOW_W = 96;
  const MIN_PILL_WINDOW_H = 54;
  const PILL_STEP_W = isLinux ? 8 : 24;
  const windowWidthFor = (contentW: number) =>
    Math.max(Math.ceil((contentW + PILL_PAD_W) / PILL_STEP_W) * PILL_STEP_W, MIN_PILL_WINDOW_W);
  const windowHeightFor = (contentH: number) =>
    Math.max(Math.round(contentH + PILL_PAD_H), MIN_PILL_WINDOW_H);
  let clusterEl: HTMLDivElement | null = null;
  let lastSentWidth = 0;
  let lastSentHeight = 0;

  let pillSizeInFlight = false;
  let pillSizeInFlightTarget: { w: number; h: number } | null = null;
  let pillSizePending: { w: number; h: number } | null = null;
  // Window size an in-progress growth (see growPillWindow) must not drop
  // below, so a settle report from before the content grew cannot queue a
  // shrink behind the growth and undo it before the transition starts.
  let pillSizeFloor: { w: number; h: number } | null = null;
  // Resolvers waiting for the native window to finish growing.
  let pillSizeWaiters: Array<() => void> = [];

  function sendPillSize(w: number, h: number) {
    pillSizeInFlight = true;
    pillSizeInFlightTarget = { w, h };
    import('@tauri-apps/api/core')
      .then(({ invoke }) => invoke('set_pill_size', { width: w, height: h }))
      .then(async () => {
        if (LINUX_INPUT_REGION) {
          const deadline = performance.now() + 750;
          while ((window.innerWidth !== w || window.innerHeight !== h) && performance.now() < deadline) {
            await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
          }
          await reportHitRect(true);
        }
        lastSentWidth = w;
        lastSentHeight = h;
      })
      .catch(() => {})
      .finally(() => {
        pillSizeInFlight = false;
        pillSizeInFlightTarget = null;
        if (pillSizePending) {
          const next = pillSizePending;
          pillSizePending = null;
          sendPillSize(next.w, next.h);
        } else {
          const waiters = pillSizeWaiters;
          pillSizeWaiters = [];
          for (const resolve of waiters) resolve();
        }
      });
  }

  function reportPillSize(width: number, height: number) {
    const w = Math.max(windowWidthFor(width), pillSizeFloor?.w ?? 0);
    const h = Math.max(windowHeightFor(height), pillSizeFloor?.h ?? 0);
    if (pillSizeInFlight) {
      pillSizePending =
        pillSizeInFlightTarget?.w === w && pillSizeInFlightTarget.h === h ? null : { w, h };
      return;
    }
    if (w === lastSentWidth && h === lastSentHeight) return;
    sendPillSize(w, h);
  }

  // Grows the native window to hold `width` x `height` and resolves once it
  // has, so a CSS transition into that space is never clipped by a window that
  // is still catching up. The timeout is only a backstop: a slow or failed
  // resize must not strand the animation.
  function growPillWindow(width: number, height: number): Promise<void> {
    // Reserve the size first so no queued or in-flight smaller report can undo
    // the growth, then wait for the queue to drain onto it.
    pillSizeFloor = {
      w: Math.max(windowWidthFor(width), pillSizeFloor?.w ?? 0),
      h: Math.max(windowHeightFor(height), pillSizeFloor?.h ?? 0),
    };
    reportPillSize(width, height);
    if (!pillSizeInFlight) return Promise.resolve();
    return new Promise((resolve) => {
      const timer = setTimeout(resolve, 150);
      pillSizeWaiters.push(() => { clearTimeout(timer); resolve(); });
    });
  }

  function measureAndResize() {
    if (!clusterEl) return;
    void reportHitRect();
    const contentW = Math.max(clusterEl.offsetWidth, state === 'error' && errOpen ? errWidth : 0);
    const contentH = Math.max(clusterEl.offsetHeight, state === 'error' && errOpen ? errHeight : 0);
    const w = windowWidthFor(contentW);
    const h = windowHeightFor(contentH);
    if (w > lastSentWidth || h > lastSentHeight) {
      reportPillSize(contentW, contentH);
    }
    timers.set('settleTimer', () => {
      if (clusterEl) reportPillSize(clusterEl.offsetWidth, clusterEl.offsetHeight);
    }, 100);
  }

  const LINUX_INPUT_REGION = isLinux && !isAndroid;
  let lastHitRect = '';
  let hitRectChain: Promise<void> = Promise.resolve();

  async function sendHitRect(force: boolean) {
    if (!clusterEl) return;
    const pill = clusterEl.querySelector<HTMLElement>('.pill');
    const box = clusterEl.getBoundingClientRect();
    const x = Math.max(0, box.left + (pill?.offsetLeft ?? 0));
    const y = Math.max(0, box.top + (pill?.offsetTop ?? 0));
    const width = Math.max(0, Math.min(window.innerWidth - x, pill?.offsetWidth ?? 0));
    const height = Math.max(0, Math.min(window.innerHeight - y, pill?.offsetHeight ?? 0));
    const key = [x, y, width, height].map(Math.round).join(',');
    if (!force && key === lastHitRect) return;
    lastHitRect = key;
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('set_pill_hit_rect', { x, y, width, height });
    } catch {
      lastHitRect = '';
    }
  }

  function reportHitRect(force = false): Promise<void> {
    if (!LINUX_INPUT_REGION) return Promise.resolve();
    hitRectChain = hitRectChain.then(() => sendHitRect(force)).catch(() => {});
    return hitRectChain;
  }

  let pillResizeObserver: ResizeObserver | null = null;

  function observeCapsule(node: HTMLElement) {
    const observer = new ResizeObserver(() => { void reportHitRect(); });
    observer.observe(node);
    return { destroy: () => observer.disconnect() };
  }

  function alignPillBars(node: HTMLElement, initialScale: number) {
    let scale = initialScale;
    let y = 0;
    const offsets = new WeakMap<HTMLElement, number>();
    let frame = 0;
    const align = () => {
      frame = 0;
      const bars = node.querySelectorAll<HTMLElement>('.bar');
      if (!bars.length) return;
      const boxes = Array.from(bars, (bar) => bar.getBoundingClientRect());
      const box = boxes[0];
      const center = box.top + box.height / 2 - y;
      y = (Math.round(center * scale) - center * scale) / scale;
      bars.forEach((bar, i) => {
        const left = boxes[i].left - (offsets.get(bar) ?? 0);
        const x = (Math.round(left * scale) - left * scale) / scale;
        offsets.set(bar, x);
        bar.style.setProperty('--bar-align-x', `${x}px`);
      });
      node.style.setProperty('--bar-align-y', `${y}px`);
    };
    const schedule = () => {
      if (!frame) frame = requestAnimationFrame(align);
    };
    const capsule = node.closest('.pill');
    const observer = new ResizeObserver(schedule);
    if (capsule) observer.observe(capsule);
    capsule?.addEventListener('animationend', schedule);
    capsule?.addEventListener('transitionend', schedule);
    window.addEventListener('resize', schedule);
    schedule();
    return {
      update(nextScale: number) { scale = nextScale; schedule(); },
      destroy() {
        observer.disconnect();
        capsule?.removeEventListener('animationend', schedule);
        capsule?.removeEventListener('transitionend', schedule);
        window.removeEventListener('resize', schedule);
        if (frame) cancelAnimationFrame(frame);
      },
    };
  }

  let dpr = typeof window !== 'undefined' ? (window.devicePixelRatio || 1) : 1;
  let monitorScale: number | null = null;
  $: barSizing = pillBarSizing(monitorScale ?? dpr);
  $: barScale = monitorScale ?? dpr;
  $: barW = barSizing.width;
  $: barGap = barSizing.gap;

  $: hfExpandedW = 12 * barW + 11 * barGap + 54;
  const LOADING_LOCAL_W = 144;
  $: processingSteadyW = Math.max(
    stageMaxW + 24,
    prevState === 'handsfree' ? hfExpandedW : 0,
    prevState === 'loading_local_model' ? LOADING_LOCAL_W : 0
  );

  let mq: MediaQueryList | null = null;
  const onDprChange = () => { dpr = window.devicePixelRatio || 1; armDprWatch(); };
  function armDprWatch() {
    if (typeof window === 'undefined') return;
    mq?.removeEventListener('change', onDprChange);
    mq = window.matchMedia(`(resolution: ${dpr}dppx)`);
    mq.addEventListener('change', onDprChange);
  }

  function refreshDpr() {
    if (typeof window === 'undefined') return;
    const live = window.devicePixelRatio || 1;
    if (live !== dpr) {
      dpr = live;
      armDprWatch();
    }
  }

  const BAR_MIN_H = 3;
  let barHeights: number[] = Array(BARS).fill(BAR_MIN_H);
  const visualizer = createPillVisualizer();
  let rafId = 0;

  function animateBars(time: number) {
    refreshDpr();
    barHeights = visualizer.step(time);
    rafId = requestAnimationFrame(animateBars);
  }

  function resetVisualizer() {
    visualizer.reset();
    barHeights = Array(BARS).fill(BAR_MIN_H);
  }

  function startRaf() {
    if (rafId === 0) { resetVisualizer(); rafId = requestAnimationFrame(animateBars); }
  }
  function stopRaf() {
    if (rafId !== 0) { cancelAnimationFrame(rafId); rafId = 0; resetVisualizer(); }
  }

  function goIdle() {
    if (dying) return;
    dying = true;
    void setPillInteractive(false);
    timers.set('dyingTimer', () => {
      dying = false;
      prevState = state;
      state = 'idle';
      clearStage();
      contextLabel = null;
      clearAudioStatus();
      resetVisualizer();
      errOpen = false;
      errWidth = 0;
      errHeight = 34;
      errLines = 1;
      errScroll = false;
      errorMsg = '';
      pillSizeFloor = null;
      timers.clear('errorTimer');
      cancelOpen = false;
      showCancelBtn = false;
      timers.clear('cancelBtnTimer');
      timers.clear('cancelDismissTimer');
      showCopyBtn = false;
      copied = false;
      timers.clear('copyBtnTimer');
      timers.clear('pasteFailedDismissTimer');
      timers.clear('copiedTimer');
      timers.clear('copiedPillTimer');
      import('@tauri-apps/api/core')
        .then(({ invoke }) => invoke('hide_dictation_pill'))
        .catch(() => {});
    }, 200);
  }

  const ERROR_COLLAPSED_WIDTH = 28;
  const ERROR_CHROME_W = 80;
  const ERROR_TEXT_MAX_W = 250;
  const ERROR_LINE_H = 15;
  const ERROR_MAX_LINES = 7;
  const ERROR_CARD_PAD_V = 22;
  const ERROR_SINGLE_H = 34;

  let openErrorTaskId = 0;
  async function openError() {
    const taskId = ++openErrorTaskId;
    const wasOpen = errOpen;
    if (!wasOpen) {
      errOpen = false;
      errWidth = ERROR_COLLAPSED_WIDTH;
      errHeight = ERROR_SINGLE_H;
      errLines = 1;
      errScroll = false;
    }
    await tick();
    if (taskId !== openErrorTaskId || state !== 'error') return;

    const applySize = async () => {
      const box = errSizerEl?.getBoundingClientRect();
      const textW = box ? Math.ceil(box.width) : 0;
      const lines = box ? Math.max(1, Math.round(box.height / ERROR_LINE_H)) : 1;

      let width = ERROR_COLLAPSED_WIDTH;
      let height = ERROR_SINGLE_H;
      let scroll = false;
      if (textW > 0 && lines <= 1) {
        width = textW + ERROR_CHROME_W;
      } else if (textW > 0) {
        width = ERROR_TEXT_MAX_W + ERROR_CHROME_W;
        height = Math.min(lines, ERROR_MAX_LINES) * ERROR_LINE_H + ERROR_CARD_PAD_V;
        scroll = lines > ERROR_MAX_LINES;
      }
      // Grow the native window first and only then start the CSS transition
      // into the new space.
      await growPillWindow(width, height);
      if (taskId !== openErrorTaskId || state !== 'error') return;
      errWidth = width;
      errHeight = height;
      errScroll = scroll;
      errLines = lines;
      errOpen = true;
      void setPillInteractive(true);
    };

    if (wasOpen) {
      applySize();
    } else {
      requestAnimationFrame(() => {
        if (taskId !== openErrorTaskId || state !== 'error') return;
        applySize();
      });
    }

    document.fonts?.ready
      .then(() => {
        if (taskId !== openErrorTaskId || state !== 'error') return;
        applySize();
      })
      .catch(() => {});
  }

  onMount(() => {
    const listeners = createListenerScope();
    let mounted = true;
    const onWindowResize = () => { void reportHitRect(true); };
    window.addEventListener('resize', onWindowResize);

    armDprWatch();

    if (clusterEl && typeof ResizeObserver !== 'undefined') {
      pillResizeObserver = new ResizeObserver(() => measureAndResize());
      pillResizeObserver.observe(clusterEl);
      // WebKitGTK can defer ResizeObserver delivery while a transparent
      // window is hidden. Seed the native size immediately so the first
      // reveal is not stranded at the 200x200 creation size.
      measureAndResize();
    }

    (async () => {
      const { listen: nativeListen } = await import('@tauri-apps/api/event');
      if (!mounted) return;
      const listen: typeof nativeListen = (event, handler, options) => {
        const registration = nativeListen(event, handler, options);
        registrations.push(listeners.track(registration));
        return registration;
      };
      const registrations: Promise<void>[] = [];

      void listen<number | null>('pill-monitor-scale', (ev) => {
        const scale = ev.payload;
        monitorScale = typeof scale === 'number' && Number.isFinite(scale) && scale >= 0.5 && scale <= 8 ? scale : null;
      });

      function applyPillState(payload: string) {
        const incoming = (payload as PillState) || 'idle';
        timers.clear('hfTimer');

        if (
          incoming !== 'processing' &&
          incoming !== 'loading_local_model'
        ) {
          clearStage();
        }
        if (isAudioState(incoming)) {
          if (incoming === 'recording') contextLabel = null;
          if (!isAudioState(state)) resetAudioStatus();
        } else if (isTerminalState(incoming)) {
          contextLabel = null;
        }
        if (!isAudioState(incoming) && incoming !== 'idle') {
          clearAudioStatus();
        }

        if (incoming === 'idle' && state !== 'idle') {
          stopRaf();
          goIdle();
          return;
        }

        if (timers.has('dyingTimer')) { timers.clear('dyingTimer'); dying = false; }

        if (incoming === 'handsfree') {
          showHfButtons = false;
          timers.set('hfTimer', () => {
            if (state === 'handsfree') {
              showHfButtons = true;
              void setPillInteractive(true);
            }
          }, 150);
        }
        prevState = state;
        state = incoming;
        if (incoming !== 'error') pillSizeFloor = null;
        void setPillInteractive(isTerminalState(incoming));
        if (isAudioState(state)) {
          refreshDpr();
          startRaf();
        } else {
          stopRaf();
        }
        if (!isAudioState(state)) resetVisualizer();
        if (state === 'error') {
          openError();
          timers.set('errorTimer', () => {
            if (state === 'error') goIdle();
          }, 10000);
        } else if (state !== 'copied' && state !== 'clipboard_warning' && state !== 'paste_failed') {
          timers.clear('errorTimer');
          errorMsg = '';
          errOpen = false;
          errWidth = 0;
          errHeight = 34;
          errLines = 1;
          errScroll = false;
        }

        if (isCancelLike(state)) {
          cancelOpen = false;
          showCancelBtn = false;
          requestAnimationFrame(() => {
            if (isCancelLike(state)) cancelOpen = true;
          });
          timers.clear('cancelBtnTimer');
          timers.set('cancelBtnTimer', () => {
            if (isCancelLike(state)) showCancelBtn = true;
          }, 150);
          timers.set('cancelDismissTimer', () => {
            if (isCancelLike(state)) goIdle();
          }, 10000);
        } else {
          timers.clear('cancelBtnTimer', 'cancelDismissTimer');
          cancelOpen = false;
          showCancelBtn = false;
        }

        if (state === 'paste_failed') {
          showCopyBtn = false;
          copied = false;
          timers.clear('copiedTimer');
          timers.clear('copyBtnTimer');
          timers.set('copyBtnTimer', () => {
            if (state === 'paste_failed') {
              showCopyBtn = true;
              void setPillInteractive(true);
            }
          }, 180);
          timers.set('pasteFailedDismissTimer', () => {
            if (state === 'paste_failed') goIdle();
          }, 10000);
        } else {
          timers.clear('copyBtnTimer', 'pasteFailedDismissTimer', 'copiedTimer');
          showCopyBtn = false;
        }

        if (state === 'copied' || state === 'clipboard_warning') {
          timers.set('copiedPillTimer', () => {
            if (state === 'copied' || state === 'clipboard_warning') goIdle();
          }, 5000);
        } else timers.clear('copiedPillTimer');
      }

      function applyPillMessage(message: string) {
        errorMsg = message;
        if (state === 'error') {
          openError();
        }
      }

      const receiveSnapshot = createPillSnapshotReceiver({
        state: applyPillState,
        message: (message) => applyPillMessage(message ?? ''),
        context: (context, incoming) => {
          if (incoming === 'idle' && context === null) return;
          contextLabel = context?.trim() || null;
        },
        stage: (stage) => stage === null ? clearStage() : onPillStage(stage),
      });
      void listen<unknown>('pill-snapshot', (ev) => receiveSnapshot(ev.payload));
      void listen<string>('pill-state', (ev) => applyPillState(ev.payload));
      void listen<string>('pill-error', (ev) => applyPillMessage(ev.payload ?? ''));

      void listen<number[]>('audio-envelope', (ev) => {
        if (ev.payload?.length) visualizer.pushEnvelope(ev.payload);
      });

      void listen<number>('audio-level-raw', (ev) => {
        onRawAudioLevel(ev.payload ?? 0);
      });

      void listen('pill-speech-detected', () => {
        onSpeechDetected();
      });

      void listen<string>('pill-context', (ev) => {
        const name = ev.payload?.trim();
        contextLabel = name ? name : null;
      });

      void listen<string>('pill-stage', (ev) => {
        onPillStage(ev.payload);
      });

      void listen('verenu:cancelled-capture-cleared', () => {
        if (isCancelLike(state)) goIdle();
      });

      try {
        await Promise.all(registrations);
      } catch {
        listeners.dispose();
        console.error('Failed to install pill event listeners.');
        return;
      }

      const { invoke } = await import('@tauri-apps/api/core');
      if (mounted) {
        await invoke('frontend_ready').catch((error) => {
          console.error('Failed to complete pill startup handshake:', error);
        });
      }

    })();

    return () => {
      mounted = false;
      pillResizeObserver?.disconnect();
      pillResizeObserver = null;
      window.removeEventListener('resize', onWindowResize);
      mq?.removeEventListener('change', onDprChange);
      cancelAnimationFrame(rafId);
      timers.clearAll();
      listeners.dispose();
    };
  });

  async function confirmHandless() {
    const { invoke } = await import('@tauri-apps/api/core');
    await invoke('stop_handless_mode').catch(() => {});
  }

  async function cancelHandless() {
    const { invoke } = await import('@tauri-apps/api/core');
    await invoke('stop_recording').catch(() => { goIdle(); });
  }

  async function retryFailed() {
    timers.clear('errorTimer');
    const { invoke } = await import('@tauri-apps/api/core');
    await invoke('retry_transcription').catch(() => {
      if (state === 'error') goIdle();
    });
  }

  function dismissError() {
    timers.clear('errorTimer');
    goIdle();
  }

  async function continueCancelled() {
    timers.clear('cancelDismissTimer');
    const { invoke } = await import('@tauri-apps/api/core');
    await invoke('resume_cancelled_capture').catch(() => { goIdle(); });
  }

  async function dismissCancelled() {
    timers.clear('cancelDismissTimer');
    goIdle();
    const { invoke } = await import('@tauri-apps/api/core');
    await invoke('dismiss_cancelled_capture').catch(() => {});
  }

  async function copyPasteFailure() {
    const { invoke } = await import('@tauri-apps/api/core');
    try {
      await invoke('copy_paste_failure_to_clipboard');
      copied = true;
      timers.clear('pasteFailedDismissTimer');
      timers.set('copiedTimer', () => {
        if (state === 'paste_failed') goIdle();
      }, 1500);
    } catch {
    }
  }

  function dismissPasteFailed() {
    timers.clear('copyBtnTimer', 'pasteFailedDismissTimer', 'copiedTimer');
    goIdle();
  }

  function dismissCopied() {
    timers.clear('copiedPillTimer');
    goIdle();
  }

  $: dumpExtras = {
    pillState: state,
    prevState,
    errorMsg,
    contextLabel,
    audioStatus,
    speechDetected,
    stageIndex,
    seenStages,
    errOpen,
    errLines,
    showHfButtons,
    cancelOpen,
  };

</script>

<div class="wrap" style="--bar-w:{barW}px; --bar-gap:{barGap}px; --stage-w:{stageW}px; --processing-steady-w:{processingSteadyW}px; --hf-expanded-w:{hfExpandedW}px">
  <div class="pill-cluster"
       class:steady-width={state === 'processing'}
       class:steady-width-hf={state === 'handsfree'}
       class:steady-width-cancel={isCancelLike(state)}
       bind:this={clusterEl}>
  {#if recordingAudioActive || (contextLabel && (state === 'recording' || state === 'processing' || state === 'handsfree' || state === 'loading_local_model'))}
      <span
        class="pill-context"
        class:status-only={recordingAudioActive && !contextLabel}
        class:audio-healthy={recordingAudioActive && audioStatus === 'healthy'}
        class:audio-weak={recordingAudioActive && audioStatus === 'weak'}
        class:audio-unheard={recordingAudioActive && audioStatus === 'unheard'}
        class:audio-muted={recordingAudioActive && audioStatus === 'muted'}
        title={pillContextTitle}
        aria-label={pillContextTitle}
        out:fly={{ y: -4, duration: reducedMotion() ? 0 : 160 }}
      >
        {#if recordingAudioActive || audioIconEverShown}
          <span class="pill-audio-status-track" class:audio-collapsed={!recordingAudioActive} aria-hidden="true">
            {#key audioStatus}
              <span
                class="pill-audio-status"
                in:fly={{ y: -7, duration: reducedMotion() ? 0 : 190, easing: rollEase }}
                out:fly={{ y: 7, duration: reducedMotion() ? 0 : 150, easing: rollEase }}
              >
                {#if audioStatus === 'healthy'}
                  <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round">
                    <polyline points="20 6 9 17 4 12"/>
                  </svg>
                {:else if audioStatus === 'weak'}
                  <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round">
                    <circle cx="12" cy="12" r="10"/>
                    <path d="M12 8v4M12 16h.01"/>
                  </svg>
                {:else if audioStatus === 'muted'}
                  <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <rect x="9" y="4" width="6" height="9" rx="3"/>
                    <path d="M5 11a7 7 0 0 0 14 0M12 18v2M9 20h6M4 4l16 16"/>
                  </svg>
                {:else if audioStatus === 'unheard'}
                  <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round">
                    <path d="M18 6 6 18M6 6l12 12"/>
                  </svg>
                {:else}
                  <span class="pill-audio-dot"></span>
                {/if}
              </span>
            {/key}
          </span>
        {/if}
        {#if contextLabel}<span class="pill-context-label">{contextLabel}</span>{/if}
      </span>
    {/if}

  {#if state === 'recording'}
    <div class="pill recording" use:alignPillBars={barScale} class:dying={dying}>
      {#each barHeights as h, i (i)}
        <div class="bar" style:height="{pillBarHeight(h, barScale, barW)}px"></div>
      {/each}
    </div>

  {:else if state === 'processing'}
    <div class="pill processing"
         class:from-rec={prevState === 'recording'}
         class:from-hf={prevState === 'handsfree'}
         class:from-loading={prevState === 'loading_local_model'}
         class:dying={dying}>
      <div class="stage-counter" aria-hidden="true">
        <div class="stage-sizer" use:measureStageRows aria-hidden="true">
          {#each STAGE_ROWS as label (label)}
            <span class="stage-row" data-stage-label={label}>{label}</span>
          {/each}
        </div>
        <div class="stage-roll" style="transform: translateY({rollPos * -STAGE_ROW_H}px)">
          {#each activeStageRows as label (label)}
            <span class="stage-row">
              <span class="stage-label">{label}</span>
              <span class="stage-label stage-shine" aria-hidden="true">{label}</span>
            </span>
          {/each}
        </div>
      </div>
    </div>

  {:else if state === 'loading_local_model'}
    <div class="pill loading-local" class:from-processing={prevState === 'processing'} class:dying={dying}>
      <span class="stage-row loading-row">
        <span class="stage-label">Loading model…</span>
        <span class="stage-label stage-shine" aria-hidden="true">Loading model…</span>
      </span>
    </div>

  {:else if state === 'error'}
    <span class="err-sizer" bind:this={errSizerEl} aria-hidden="true">{displayError}</span>
    <div class="pill error" class:err-open={errOpen} class:err-card={errLines > 1} class:err-scroll={errScroll} class:dying={dying}
         style={errWidth ? `width:${errWidth}px; height:${errHeight}px` : ''}>
      {#if errOpen}
        <button class="hf-btn err-dismiss" onclick={dismissError} aria-label="Dismiss">
          <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round">
            <path d="M6 6l12 12M6 18 18 6"/>
          </svg>
        </button>
      {/if}
      <span class="err-text" bind:this={errTextEl}>{displayError}</span>
      {#if errOpen}
        <button class="hf-btn err-retry" onclick={retryFailed} aria-label="Retry">
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M21 12a9 9 0 1 1-2.64-6.36"/><path d="M21 4v5h-5"/>
          </svg>
        </button>
      {/if}
    </div>

  {:else if isCancelLike(state)}
    <div class="pill cancelled"
         class:interrupted={state === 'interrupted'}
         class:cancel-open={cancelOpen}
         class:dying={dying}
         class:from-rec={prevState === 'recording'}
         class:from-hf={prevState === 'handsfree'}>
      <button class="hf-btn cancel" onclick={dismissCancelled} aria-label="Dismiss">
        <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round">
          <path d="M6 6l12 12M6 18 18 6"/>
        </svg>
      </button>
      <span class="cancel-text">{state === 'interrupted' ? 'Interrupted' : 'Cancelled'}</span>
      <button class="hf-btn confirm" class:btn-visible={showCancelBtn} disabled={!showCancelBtn} tabindex={showCancelBtn ? 0 : -1} aria-hidden={!showCancelBtn} onclick={continueCancelled} aria-label="Undo — keep recording">
        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M9 14 4 9l5-5"/><path d="M4 9h10.5a5.5 5.5 0 0 1 5.5 5.5v0a5.5 5.5 0 0 1-5.5 5.5H11"/>
        </svg>
      </button>
    </div>

  {:else if state === 'paste_failed'}
    <div class="pill paste-failed" class:copy-open={showCopyBtn} class:dying={dying}>
      <svg class="err-icon" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <circle cx="12" cy="12" r="10"/><path d="M12 8v4M12 16h.01"/>
      </svg>
      <span class="paste-failed-text">{errorMsg === 'Partly pasted' ? 'Partly pasted' : 'Not pasted'}</span>
      {#if showCopyBtn}
        <button class="hf-btn copy-btn" onclick={copyPasteFailure} aria-label="Copy to clipboard">
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
            {#if copied}
              <polyline points="20 6 9 17 4 12"/>
            {:else}
              <rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/>
            {/if}
          </svg>
        </button>
        <button class="hf-btn pf-dismiss" onclick={dismissPasteFailed} aria-label="Dismiss">
          <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round">
            <path d="M6 6l12 12M6 18 18 6"/>
          </svg>
        </button>
      {/if}
    </div>

  {:else if state === 'copied'}
    <div class="pill copied" class:dying={dying}>
      <svg class="copied-icon" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
        <polyline points="20 6 9 17 4 12"/>
      </svg>
      <span class="copied-text">{errorMsg || 'Copied last dictation to clipboard'}</span>
      <button class="hf-btn copied-dismiss" onclick={dismissCopied} aria-label="Dismiss">
        <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round">
          <path d="M6 6l12 12M6 18 18 6"/>
        </svg>
      </button>
    </div>

  {:else if state === 'clipboard_warning'}
    <div class="pill copied clipboard-warning" class:dying={dying}>
      <svg class="copied-icon" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
        <circle cx="12" cy="12" r="10"/>
        <path d="M12 8v4"/>
        <circle cx="12" cy="16" r="1" fill="currentColor" stroke="none"/>
      </svg>
      <span class="copied-text">{errorMsg || 'Clipboard phrase skipped'}</span>
      <button class="hf-btn copied-dismiss" onclick={dismissCopied} aria-label="Dismiss"><svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2"><path d="M6 6l12 12M6 18 18 6"/></svg></button>
    </div>

  {:else if state === 'handsfree'}
    <div class="pill handsfree" use:observeCapsule class:dying={dying} class:hf-expanded={showHfButtons && !dying} class:no-anim={prevState === 'recording'}>
      {#if showHfButtons}
        <button class="hf-btn cancel" onclick={cancelHandless} aria-label="Cancel">
          <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round">
            <path d="M6 6l12 12M6 18 18 6"/>
          </svg>
        </button>
      {/if}
      <div class="bars-hf" use:alignPillBars={barScale}>
        {#each barHeights as h, i (i)}
          <div class="bar" style:height="{pillBarHeight(h, barScale, barW)}px"></div>
        {/each}
      </div>
      {#if showHfButtons}
        <button class="hf-btn confirm" onclick={confirmHandless} aria-label="Confirm">
          <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
            <path d="M20 6L9 17l-5-5"/>
          </svg>
        </button>
      {/if}
    </div>
  {/if}
  </div>

</div>
<AgentAccessibilityDump windowKind="pill" extras={dumpExtras} />

<style>
  :global(*, *::before, *::after) { box-sizing: border-box; }

  :global(:root:root) { color-scheme: normal; }

  :global(html, body, #pill-root) {
    margin: 0; padding: 0;
    background: transparent !important;
    background-color: rgba(0, 0, 0, 0) !important;
    overflow: hidden;
    width: 100vw; height: 100vh;
    font-family: var(--sans);
  }

  :global(*) {
    user-select: none;
    -webkit-user-select: none;
  }
  :global(*:focus),
  :global(*:focus-visible) {
    outline: none;
  }

  .wrap {
    width: 100vw; height: 100vh;
    display: flex;
    align-items: flex-end;
    justify-content: center;
    padding-bottom: 10px;
  }

  .pill-cluster {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    position: relative;
  }
  .pill-cluster.steady-width {
    min-width: var(--processing-steady-w, 144px);
  }
  .pill-cluster.steady-width-hf {
    min-width: var(--hf-expanded-w, 132px);
  }
  .pill-cluster.steady-width-cancel {
    min-width: 172px;
  }
   .pill-context {
     display: inline-flex;
     align-items: center;
     justify-content: center;
     font-size: 10.5px;
     font-weight: 500;
     line-height: 1;
    letter-spacing: 0.02em;
    color: var(--pill-context-fg);
    background: var(--pill-context-bg);
    box-shadow: 0 0 0 1px var(--pill-context-line) inset;
    border-radius: 999px;
    padding: 3px 7px;
    white-space: nowrap;
    max-width: 180px;
     overflow: hidden;
     text-overflow: ellipsis;
     animation: chipIn 0.18s ease-out both;
     transition: color var(--ui-duration-fast, 150ms), background-color var(--ui-duration-fast, 150ms), box-shadow var(--ui-duration-fast, 150ms);
   }
   .pill-context.status-only {
     min-width: 10px;
     padding-inline: 5px;
   }
   .pill-context-label {
     min-width: 0;
     overflow: hidden;
     text-overflow: ellipsis;
   }
   .pill-audio-status-track {
     position: relative;
     width: 10px;
     height: 10px;
     margin-right: 2px;
     overflow: hidden;
     transition: width 0.24s cubic-bezier(0.22, 1, 0.36, 1), margin-right 0.24s cubic-bezier(0.22, 1, 0.36, 1);
   }
   .pill-audio-status-track.audio-collapsed {
     width: 0;
     margin-right: 0;
   }
   .pill-audio-status {
     position: absolute;
     inset: 0;
     display: flex;
     align-items: center;
     justify-content: center;
     color: var(--accent);
     transition: transform 0.24s cubic-bezier(0.22, 1, 0.36, 1), opacity 0.2s ease;
   }
   .pill-audio-status-track.audio-collapsed .pill-audio-status {
     transform: translateX(-100%);
     opacity: 0;
   }
   .pill-audio-status svg {
     display: block;
   }
   .pill-audio-dot {
     width: 4px;
     height: 4px;
     border-radius: 50%;
     background: currentColor;
   }
   .pill-context.audio-healthy .pill-audio-status {
     color: var(--accent);
   }
   .pill-context.audio-weak .pill-audio-status {
     color: var(--warning);
   }
   .pill-context.audio-unheard .pill-audio-status {
     color: var(--danger);
   }
   .pill-context.audio-muted {
     color: var(--danger);
     background: var(--danger-bg);
     box-shadow: 0 0 0 1px var(--danger-line) inset;
   }
   .pill-context.audio-muted .pill-audio-status {
     color: currentColor;
   }
   @media (prefers-reduced-motion: reduce) {
     .pill-context,
     .pill-audio-status-track,
     .pill-audio-status {
       animation: none !important;
       transition: none !important;
     }
   }
  @keyframes chipIn {
    from { opacity: 0; transform: translateY(-3px); }
    to   { opacity: 1; transform: translateY(0); }
  }

  .pill {
    background: var(--pill-bg);
    color: var(--pill-fg);
    border-radius: 999px;
    height: 34px;
    display: flex;
    align-items: center;
    justify-content: center;
    box-shadow: 0 0 0 1px var(--pill-line) inset;
    animation: pillIn 0.22s cubic-bezier(0.34, 1.56, 0.64, 1) both;
  }

  @keyframes pillIn {
    from { transform: translateY(6px) scale(0.94); opacity: 0; }
    to   { transform: translateY(0) scale(1); opacity: 1; }
  }

  @keyframes pillOut {
    from { transform: translateY(0) scale(1); opacity: 1; }
    to   { transform: translateY(5px) scale(0.9); opacity: 0; }
  }

  .pill.recording.dying,
  .pill.handsfree.dying,
  .pill.error.dying,
  .pill.cancelled.dying,
  .pill.paste-failed.dying,
  .pill.copied.dying,
  .pill.processing.dying,
  .pill.loading-local.dying {
    animation: pillOut 0.18s cubic-bezier(0.4, 0, 1, 1) both;
    pointer-events: none;
  }

  .pill.no-anim { animation: none; }

  .pill.recording {
    gap: var(--bar-gap, 2px);
    padding: 0 7px;
    width: calc(12 * var(--bar-w, 3px) + 11 * var(--bar-gap, 2px) + 14px);
    animation: pillIn 0.22s cubic-bezier(0.34, 1.56, 0.64, 1) 0.25s both;
  }

  .bar {
    width: var(--bar-w, 3px);
    background: var(--pill-bar);
    border-radius: 999px;
    flex-shrink: 0;
    transform: translate(var(--bar-align-x, 0px), var(--bar-align-y, 0px));
  }

  .pill.processing {
    width: calc(var(--stage-w, 116px) + 24px);
    padding: 0 12px;
    position: relative;
    transition: width 0.26s cubic-bezier(0.22, 1, 0.36, 1);
  }
  .pill.loading-local { width: 144px; padding: 0 14px; justify-content: center; }

  .pill.processing.from-rec {
    animation: processIn 0.28s cubic-bezier(0.22, 1, 0.36, 1) backwards,
               pillIn 0.22s cubic-bezier(0.34, 1.56, 0.64, 1) both;
  }
  @keyframes processIn {
    from { width: calc(12 * var(--bar-w, 3px) + 11 * var(--bar-gap, 2px) + 14px); }
  }

  .pill.processing.from-hf {
    animation: processFromHf 0.25s ease-out backwards,
               pillIn 0.22s cubic-bezier(0.34, 1.56, 0.64, 1) both;
  }
  @keyframes processFromHf {
    from { width: calc(12 * var(--bar-w, 3px) + 11 * var(--bar-gap, 2px) + 54px); }
  }

  .pill.loading-local.from-processing {
    animation: loadingLocalIn 0.26s cubic-bezier(0.22, 1, 0.36, 1) both;
  }
  @keyframes loadingLocalIn {
    from { width: calc(var(--stage-w, 116px) + 24px); }
    to   { width: 144px; }
  }
  .pill.loading-local.from-processing .loading-row {
    animation: scanIn 0.16s ease 0.16s both;
  }

  .pill.processing.from-loading {
    animation: processFromLoading 0.26s ease-out backwards,
               pillIn 0.22s cubic-bezier(0.34, 1.56, 0.64, 1) both;
  }
  @keyframes processFromLoading {
    from { width: 144px; }
  }

  .pill.processing.from-rec.dying,
  .pill.processing.from-hf.dying,
  .pill.processing.from-loading.dying,
  .pill.loading-local.from-processing.dying {
    animation: pillOut 0.18s cubic-bezier(0.4, 0, 1, 1) both;
  }

  .stage-counter {
    flex: 1;
    height: 16px;
    overflow: hidden;
    display: flex;
    justify-content: center;
    position: relative;
  }
  .stage-sizer {
    position: absolute;
    top: 0;
    left: 0;
    visibility: hidden;
    pointer-events: none;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
  }
  .stage-roll {
    display: flex;
    flex-direction: column;
    align-items: center;
    flex-shrink: 0;
    will-change: transform;
    transition: transform 0.28s cubic-bezier(0.22, 1, 0.36, 1);
  }
  .stage-row {
    position: relative;
    height: 16px;
    line-height: 16px;
    font-size: var(--pill-stage-size);
    font-weight: var(--pill-stage-weight);
    white-space: nowrap;
    display: block;
  }
  .stage-label {
    display: block;
    color: var(--pill-muted);
  }
  .stage-shine {
    position: absolute;
    inset: 0;
    color: transparent;
    background-image: linear-gradient(
      90deg,
      var(--pill-shine-clear) 0%,
      var(--pill-shine-mid) 25%,
      var(--pill-shine-strong) 50%,
      var(--pill-shine-mid) 75%,
      var(--pill-shine-clear) 100%
    );
    background-size: 24px 100%;
    background-repeat: no-repeat;
    background-clip: text;
    -webkit-background-clip: text;
    animation: stage-scan 1.05s ease-in-out infinite alternate;
  }
  @keyframes stage-scan {
    from { background-position: 0% 0; }
    to   { background-position: 100% 0; }
  }
  @keyframes scanIn {
    from { opacity: 0; }
    to   { opacity: 1; }
  }

  .pill.error {
    width: 42px;
    height: 34px;
    gap: 8px;
    padding: 0 14px;
    background: var(--pill-error-bg);
    color: var(--pill-error-fg);
    border-radius: 17px;
    box-shadow: 0 0 0 1px var(--pill-error-border);
    overflow: hidden;
    transition: width 0.3s cubic-bezier(0.22, 1, 0.36, 1),
                height 0.3s cubic-bezier(0.22, 1, 0.36, 1),
                padding 0.3s cubic-bezier(0.22, 1, 0.36, 1);
  }
  .err-icon {
    flex-shrink: 0;
    color: var(--pill-error-fg);
  }
  .err-text {
    font-size: 11.5px; font-weight: 500;
    line-height: 15px;
    color: var(--pill-error-fg);
    white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
    opacity: 0;
    transition: opacity 0.18s ease 0.08s;
  }
  .pill.error.err-open .err-text { opacity: 1; }

  .pill.error.err-card {
    align-items: flex-start;
    padding: 10px 14px;
  }
  .pill.error.err-card .err-text {
    flex: 1;
    white-space: normal;
    overflow-wrap: anywhere;
    text-overflow: clip;
    max-height: 100%;
  }
  /* Standard scrollbar properties only — Chromium ignores ::-webkit-scrollbar
     entirely once scrollbar-width/-color are set, so the two cannot be
     combined. The thumb is tinted from the error foreground rather than the
     border colour, which was too close to the background to read as an
     affordance that there is more text below. */
  .pill.error.err-card.err-scroll .err-text {
    overflow-y: auto;
    padding-right: 6px;
    scrollbar-width: thin;
    scrollbar-color: rgba(255, 143, 128, 0.45) transparent;
  }
  .pill.error.err-card .err-retry,
  .pill.error.err-card .err-dismiss {
    margin-top: 1px;
  }

  .err-sizer {
    position: absolute;
    visibility: hidden;
    pointer-events: none;
    width: max-content;
    max-width: 250px;
    font-size: 11.5px;
    font-weight: 500;
    line-height: 15px;
    white-space: normal;
    overflow-wrap: anywhere;
  }

  .hf-btn.err-retry {
    color: var(--pill-error-fg);
    animation: hfBtnIn 0.12s cubic-bezier(0.34, 1.56, 0.64, 1) 0.13s both;
  }
  .hf-btn.err-retry:hover { background: rgba(255,255,255,0.14); }

  .hf-btn.err-dismiss {
    color: var(--pill-error-fg);
    opacity: 0.75;
    animation: hfBtnIn 0.12s cubic-bezier(0.34, 1.56, 0.64, 1) 0.1s both;
  }
  .hf-btn.err-dismiss:hover { opacity: 1; background: rgba(255,255,255,0.14); }

  .pill.cancelled {
    width: 158px;
    gap: 8px;
    padding: 0 8px;
    border-radius: 999px;
    overflow: hidden;
  }
  .pill.cancelled.from-rec {
    animation: cancelFromRec 0.24s cubic-bezier(0.22, 1, 0.36, 1) backwards,
               pillIn 0.22s cubic-bezier(0.34, 1.56, 0.64, 1) both;
  }
  @keyframes cancelFromRec {
    from { width: calc(12 * var(--bar-w, 3px) + 11 * var(--bar-gap, 2px) + 14px); }
  }
  .pill.cancelled.from-hf {
    animation: cancelFromHf 0.24s cubic-bezier(0.22, 1, 0.36, 1) backwards,
               pillIn 0.22s cubic-bezier(0.34, 1.56, 0.64, 1) both;
  }
  @keyframes cancelFromHf {
    from { width: calc(12 * var(--bar-w, 3px) + 11 * var(--bar-gap, 2px) + 54px); }
  }
  .pill.cancelled.interrupted {
    width: 172px;
  }
  .pill.cancelled.from-rec.dying,
  .pill.cancelled.from-hf.dying {
    animation: pillOut 0.18s cubic-bezier(0.4, 0, 1, 1) both;
  }
  .cancel-text {
    font-size: 11.5px; font-weight: 500;
    white-space: nowrap; overflow: hidden;
    opacity: 0;
    flex: 1;
    text-align: center;
    transition: opacity 0.16s ease 0.05s;
  }
  .pill.cancelled.cancel-open .cancel-text { opacity: 1; }
  .pill.cancelled .hf-btn.confirm {
    animation: none;
    opacity: 0;
    pointer-events: none;
    transition: opacity 0.18s ease;
  }
  .pill.cancelled .hf-btn.confirm.btn-visible {
    opacity: 1;
    pointer-events: auto;
  }

  .pill.paste-failed {
    gap: 8px;
    padding: 0 14px;
    background: var(--pill-error-bg);
    color: var(--pill-error-fg);
    border-radius: 999px;
    box-shadow: 0 0 0 1px var(--pill-error-border);
    width: 128px;
    overflow: hidden;
    transition: width 0.26s cubic-bezier(0.22, 1, 0.36, 1),
                padding 0.26s cubic-bezier(0.22, 1, 0.36, 1);
  }
  .pill.paste-failed.copy-open {
    width: 184px;
    padding: 0 8px 0 14px;
  }
  .paste-failed-text {
    font-size: 11.5px; font-weight: 500;
    color: var(--pill-error-fg);
    white-space: nowrap; overflow: hidden;
    flex: 1;
    text-align: center;
  }
  .hf-btn.copy-btn {
    color: var(--pill-error-fg);
    animation: hfBtnIn 0.12s cubic-bezier(0.34, 1.56, 0.64, 1) both;
  }
  .hf-btn.copy-btn:hover { background: rgba(255,255,255,0.14); }

  .hf-btn.pf-dismiss {
    color: var(--pill-error-fg);
    opacity: 0.75;
    animation: hfBtnIn 0.12s cubic-bezier(0.34, 1.56, 0.64, 1) 0.03s both;
  }
  .hf-btn.pf-dismiss:hover { opacity: 1; background: rgba(255,255,255,0.14); }

  .pill.copied {
    gap: 8px;
    padding: 0 8px 0 14px;
    white-space: nowrap;
  }
  .copied-icon { color: var(--pill-fg); flex-shrink: 0; }
  .copied-text {
    font-size: 11.5px; font-weight: 500;
    white-space: nowrap;
  }
  .hf-btn.copied-dismiss {
    color: var(--pill-muted);
    animation: hfBtnIn 0.12s cubic-bezier(0.34, 1.56, 0.64, 1) 0.05s both;
  }
  .hf-btn.copied-dismiss:hover { color: var(--pill-muted-strong); background: var(--pill-hover); }

  .pill.handsfree {
    width: calc(12 * var(--bar-w, 3px) + 11 * var(--bar-gap, 2px) + 14px);
    padding: 0 7px;
    gap: 2px;
    transition: width 0.2s cubic-bezier(0.22, 1, 0.36, 1),
                padding 0.18s ease,
                gap 0.18s ease;
  }
  .pill.handsfree.hf-expanded { width: calc(12 * var(--bar-w, 3px) + 11 * var(--bar-gap, 2px) + 54px); padding: 0 5px; gap: 4px; }

  .bars-hf { display: flex; align-items: center; gap: var(--bar-gap, 2px); flex: 1; justify-content: center; }

  .hf-btn {
    width: 18px; height: 18px;
    background: transparent; border: 0;
    display: grid; place-items: center;
    flex-shrink: 0; cursor: pointer;
    border-radius: 4px;
    padding: 0;
    transition: opacity 0.15s;
  }
  .hf-btn.cancel  { color: var(--pill-muted); }
  .hf-btn.confirm { color: var(--pill-fg); }
  .hf-btn.cancel:hover  { color: var(--pill-muted-strong); }
  .hf-btn.confirm:hover { color: var(--pill-fg); }

  @keyframes hfBtnIn {
    from { opacity: 0; transform: translateY(3px); }
    to   { opacity: 1; transform: translateY(0); }
  }
  .hf-btn.cancel  { animation: hfBtnIn 0.16s ease-out both; }
  .hf-btn.confirm { animation: hfBtnIn 0.16s ease-out 0.03s both; }

  /* Reduced motion: no scale/slide/width morphs. States swap with a short
     opacity fade; continuous effects (bars, spinner, shine) carry information
     and are left alone. */
  @keyframes pillFade {
    from { opacity: 0; }
    to   { opacity: 1; }
  }
  @media (prefers-reduced-motion: reduce) {
    .pill,
    .pill.processing.from-rec,
    .pill.processing.from-hf,
    .pill.processing.from-loading,
    .pill.loading-local.from-processing,
    .pill.cancelled.from-rec,
    .pill.cancelled.from-hf {
      animation: pillFade 0.12s linear both !important;
    }
    .pill.recording { animation-delay: 0.25s !important; }
    .pill.dying,
    .pill.processing.from-rec.dying,
    .pill.processing.from-hf.dying,
    .pill.processing.from-loading.dying,
    .pill.loading-local.from-processing.dying,
    .pill.cancelled.from-rec.dying,
    .pill.cancelled.from-hf.dying {
      animation: pillFade 0.12s linear reverse both !important;
    }
    .pill.no-anim:not(.dying) { animation: none !important; }
    .pill,
    .pill.processing,
    .pill.error,
    .pill.cancelled,
    .pill.paste-failed,
    .pill.copied,
    .pill.handsfree,
    .stage-roll,
    .err-text,
    .pill-context {
      transition: none !important;
    }
    .hf-btn { animation: none !important; }
  }
</style>
