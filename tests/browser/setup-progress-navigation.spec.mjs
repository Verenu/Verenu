import { test, expect } from './fixtures.mjs';

for (const reducedMotion of ['no-preference', 'reduce']) {
  for (const mode of ['pointer', 'keyboard Enter', 'keyboard Space']) {
    test(`Setup honors an early progress step click (${mode}, ${reducedMotion})`, async ({ page, session }) => {
      const previous = {
        setup_complete: await session.invoke('get_setting', { key: 'setup_complete' }),
        setup_progress: await session.invoke('get_setting', { key: 'setup_progress' }),
      };
      const label = (n) => page.getByText(new RegExp(`^Step ${n} of \\d+`));
      try {
        await page.emulateMedia({ reducedMotion });
        await session.invoke('save_setting', { key: 'setup_complete', value: false });
        await session.invoke('save_setting', { key: 'setup_progress', value: { step: 6, provider: 'local' } });
        await page.reload();
        await expect(label(6)).toBeVisible();
        await expect(page.getByRole('button', { name: 'Next', exact: true })).toBeEnabled();

        const keyboard = mode !== 'pointer';
        const outgoing = await page.locator('.step-wrap:not([inert])').elementHandle();
        await page.clock.install({ time: new Date('2026-01-01T00:00:00Z') });
        await page.clock.pauseAt(new Date('2026-01-01T01:00:00Z'));
        await page.getByRole('button', { name: 'Next', exact: true }).evaluate(button => button.click());
        await page.clock.runFor(50);
        const early = await page.evaluate(({ keyboard, outgoing }) => {
          const button = [...document.querySelectorAll('button')].find(b => b.textContent.trim() === 'Next');
          const dot = document.querySelector('button[aria-label="Step 5"]');
          const result = { slidingAfterNext: button.disabled, landingFocusSettled: false, slidingAtDotFocus: false, dotFocused: false };
          if (!keyboard) {
            dot.click();
            return result;
          }
          const landed = document.activeElement?.closest('.step-wrap');
          result.landingFocusSettled = !!landed && landed !== outgoing;
          if (!result.landingFocusSettled) return result;
          dot.focus();
          result.slidingAtDotFocus = button.disabled;
          result.dotFocused = document.activeElement === dot;
          window.__progressDotInput = [];
          const record = (event) => window.__progressDotInput.push({
            type: event.type, key: event.key ?? null, trusted: event.isTrusted, detail: event.detail ?? null,
            nextDisabled: button.disabled, onDot: event.target === dot,
          });
          for (const type of ['keydown', 'keyup', 'click']) dot.addEventListener(type, record, { capture: true });
          return result;
        }, { keyboard, outgoing });
        expect(early.slidingAfterNext).toBe(true);
        if (keyboard) {
          expect(early.landingFocusSettled).toBe(true);
          expect(early.slidingAtDotFocus).toBe(true);
          expect(early.dotFocused).toBe(true);
          const key = mode === 'keyboard Space' ? 'Space' : 'Enter';
          await page.keyboard.press(key);
          const input = await page.evaluate(() => window.__progressDotInput);
          const clicks = input.filter(entry => entry.type === 'click');
          expect(input.filter(entry => entry.type === 'keydown').map(entry => [entry.key, entry.trusted, entry.nextDisabled, entry.onDot]))
            .toEqual([[key === 'Space' ? ' ' : 'Enter', true, true, true]]);
          expect(clicks).toHaveLength(1);
          expect(clicks[0]).toMatchObject({ trusted: true, detail: 0, nextDisabled: true, onDot: true });
        }

        await page.clock.runFor(1000);
        await page.clock.resume();
        await expect(label(5)).toBeVisible();
        await expect(page.getByRole('button', { name: 'Step 5', exact: true })).toHaveAttribute('aria-current', 'step');
        await expect(page.getByRole('button', { name: 'Next', exact: true })).toBeEnabled();
        await page.getByRole('button', { name: 'Next', exact: true }).click();
        await expect(label(6)).toBeVisible();
      } finally {
        await page.clock.resume();
        await session.invoke('save_setting', { key: 'setup_progress', value: previous.setup_progress ?? null });
        await session.invoke('save_setting', { key: 'setup_complete', value: previous.setup_complete ?? true });
        await page.emulateMedia({ reducedMotion: null });
        await page.reload();
      }
    });
  }
}
