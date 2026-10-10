import { test, expect } from './fixtures.mjs';

// Regression: progress dots stay clickable while a step slides, and a click in
// that window used to be dropped. It must land on the requested step once the
// slide ends, in both motion modes, by pointer and by keyboard (Enter and Space).
// The pointer click runs in the same task as Next. A keyboard user's focus first
// moves into the incoming step (Setup's landing focus: its heading, else its first
// control, in a new .step-wrap), so the keyboard cases focus the dot only after
// that, still mid-slide, and record the real key's origin. Focus left on the
// outgoing step does not count: the landing move would take it off the dot.
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
        // Click Next and, in the same task, the earlier dot (pointer). For the
        // keyboard, wait for landing focus to settle on the new step, then focus
        // the dot while the slide is still running.
        const early = await page.evaluate(async (keyboard) => {
          const button = [...document.querySelectorAll('button')].find(b => b.textContent.trim() === 'Next');
          const dot = document.querySelector('button[aria-label="Step 5"]');
          const outgoing = document.querySelector('.step-wrap');
          button.click();
          await new Promise(requestAnimationFrame);
          const result = { slidingAfterNext: button.disabled, landingFocusSettled: false, slidingAtDotFocus: false, dotFocused: false };
          if (!keyboard) {
            dot.click();
            return result;
          }
          for (let frame = 0; frame < 20 && button.disabled; frame++) {
            const landed = document.activeElement?.closest('.step-wrap');
            if (landed && landed !== outgoing) {
              result.landingFocusSettled = true;
              break;
            }
            await new Promise(requestAnimationFrame);
          }
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
        }, keyboard);
        expect(early.slidingAfterNext).toBe(true);
        if (keyboard) {
          expect(early.landingFocusSettled).toBe(true);
          expect(early.slidingAtDotFocus).toBe(true);
          expect(early.dotFocused).toBe(true);
          const key = mode === 'keyboard Space' ? 'Space' : 'Enter';
          await page.keyboard.press(key);
          const input = await page.evaluate(() => window.__progressDotInput);
          // Real browser key events (trusted), delivered to the dot while Next was
          // still disabled. A native button turns Enter (keydown) or Space (keyup)
          // into a keyboard click, which has detail 0 and is likewise trusted.
          const clicks = input.filter(entry => entry.type === 'click');
          expect(input.filter(entry => entry.type === 'keydown').map(entry => [entry.key, entry.trusted, entry.nextDisabled, entry.onDot]))
            .toEqual([[key === 'Space' ? ' ' : 'Enter', true, true, true]]);
          expect(clicks).toHaveLength(1);
          expect(clicks[0]).toMatchObject({ trusted: true, detail: 0, nextDisabled: true, onDot: true });
        }

        await expect(label(5)).toBeVisible();
        await expect(page.getByRole('button', { name: 'Step 5', exact: true })).toHaveAttribute('aria-current', 'step');
        // The queued request ends the transition cleanly: navigation works again.
        await expect(page.getByRole('button', { name: 'Next', exact: true })).toBeEnabled();
        await page.getByRole('button', { name: 'Next', exact: true }).click();
        await expect(label(6)).toBeVisible();
      } finally {
        await session.invoke('save_setting', { key: 'setup_progress', value: previous.setup_progress ?? null });
        await session.invoke('save_setting', { key: 'setup_complete', value: previous.setup_complete ?? true });
        await page.emulateMedia({ reducedMotion: null });
        await page.reload();
      }
    });
  }
}
