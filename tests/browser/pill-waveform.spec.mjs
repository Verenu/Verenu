import { test, expect } from './fixtures.mjs';

for (const scale of [1, 1.25, 1.5, 1.75, 2]) {
for (const mode of ['recording', 'handsfree']) {
test.describe(`pill waveform at ${scale}x`, () => {
test.use({ deviceScaleFactor: scale });
test(`${mode} aligns resting bars and renders smooth audio-driven heights`, async ({ page, session }) => {
  await page.addInitScript(() => {
    let id = 0;
    const callbacks = new Map();
    const listeners = new Map();
    window.__TAURI_INTERNALS__ = {
      transformCallback(callback) { callbacks.set(++id, callback); return id; },
      invoke(command, args) {
        if (command === 'plugin:event|listen') {
          const handlers = listeners.get(args.event) ?? [];
          handlers.push(callbacks.get(args.handler));
          listeners.set(args.event, handlers);
          return Promise.resolve(args.handler);
        }
        return Promise.resolve(null);
      },
    };
    window.emitPillTestEvent = (event, payload) => {
      for (const handler of listeners.get(event) ?? []) handler({ event, payload, id: 0 });
    };
    window.pillTestReady = () => listeners.has('pill-state') && listeners.has('audio-envelope');
  });
  await page.goto(new URL('/pill.html', session.access.localAccessUrl).toString());
  await page.waitForFunction(() => window.pillTestReady());
  await page.evaluate(mode => {
    window.emitPillTestEvent('pill-monitor-scale', window.devicePixelRatio);
    window.emitPillTestEvent('pill-state', mode);
  }, mode);
  const bars = page.locator('.pill .bar');
  await expect(bars).toHaveCount(12);
  if (mode === 'handsfree') await expect(page.locator('.pill.hf-expanded')).toBeVisible();
  await bars.first().evaluate(async node => {
    const animations = node.closest('.pill').getAnimations();
    await Promise.all(animations.map(animation => animation.finished.catch(() => {})));
  });
  const alignedAt = async factor => bars.evaluateAll((nodes, factor) => nodes.every(node => {
    const box = node.getBoundingClientRect();
    return [box.left, box.right, box.top, box.bottom].every(value =>
      Math.abs(value * factor - Math.round(value * factor)) < 0.06
    );
  }), factor);
  await expect.poll(() => alignedAt(scale)).toBe(true);
  const changedScale = scale === 1.75 ? 1.25 : 1.75;
  await page.evaluate(factor => window.emitPillTestEvent('pill-monitor-scale', factor), changedScale);
  await expect(page.locator('.bar-grid')).toHaveCount(1);
  await expect.poll(() => alignedAt(changedScale)).toBe(true);
  await page.evaluate(factor => window.emitPillTestEvent('pill-monitor-scale', factor), scale);
  await expect(page.locator('.bar-grid')).toHaveCount(0);
  await expect.poll(() => alignedAt(scale)).toBe(true);
  const resting = await bars.evaluateAll(nodes => nodes.map(node => {
    const { width, height } = node.getBoundingClientRect();
    return { width, height };
  }));
  expect(resting.every(bar => Math.abs(bar.width - resting[0].width) < 0.02)).toBe(true);
  expect(resting.every(bar => Math.abs(bar.height - resting[0].height) < 0.02)).toBe(true);
  expect(resting[0].height).toBeGreaterThanOrEqual(resting[0].width);

  await page.evaluate(() => {
    let sample = 0;
    setInterval(() => {
      const envelope = Array.from({ length: 5 }, () => 0.05 + 0.4 * Math.abs(Math.sin(sample++ * 0.13)));
      window.emitPillTestEvent('audio-envelope', envelope);
    }, 50);
  });
  await expect.poll(async () => bars.evaluateAll((nodes, restHeight) => nodes.some(node => {
    const height = node.getBoundingClientRect().height;
    return Math.abs(height - restHeight) > 0.5 && Math.abs(height / 2 - Math.round(height / 2)) > 0.05;
  }), resting[0].height)).toBe(true);
  await expect.poll(async () => bars.evaluateAll((nodes, factor) => nodes.every(node => {
    const box = node.getBoundingClientRect();
    return [box.left, box.right].every(value =>
      Math.abs(value * factor - Math.round(value * factor)) < 0.06
    );
  }), scale)).toBe(true);
});
});
}
}
