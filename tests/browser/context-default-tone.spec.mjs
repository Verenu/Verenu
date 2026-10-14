import { test, expect } from './fixtures.mjs';
import path from 'node:path';

for (const intensity of ['rules', 'none']) {
  test(`AI Context can inherit an editable default tone with global ${intensity}`, async ({ page, session }, testInfo) => {
    const previousIntensity = await session.invoke('get_setting', { key: 'cleanup_intensity' });
    const previousTone = await session.invoke('get_setting', { key: 'default_tone' });
    const context = await session.invoke('create_context', { name: 'Synthetic AI tone', contextualFormattingDisabled: false });
    try {
      await session.invoke('update_context_settings', {
        contextId: context.id, icon: null, tone: null, cleanupIntensity: 'light',
        customInstructions: null, contextualFormattingDisabled: false, pasteInChunks: null,
      });
      await session.invoke('save_setting', { key: 'cleanup_intensity', value: intensity });
      await session.invoke('save_setting', { key: 'default_tone', value: 'casual' });
      await page.reload();
      await page.locator('[data-debug-id="nav.style"]').click();
      const toneTab = page.getByRole('tab', { name: 'Personal Tone', exact: true });
      if (await toneTab.isVisible()) await toneTab.click();
      const grid = page.locator('.style-grid:visible').filter({ has: page.getByText('Formal', { exact: true }) });
      await grid.scrollIntoViewIfNeeded();
      await page.evaluate(() => document.fonts.ready);
      if (intensity === 'rules') {
        await page.screenshot({ path: path.join(path.dirname(process.env.VERENU_SESSION_ACCESS_FILE), `context-tone-${testInfo.project.name}.png`) });
      }
      await expect(grid).toHaveJSProperty('inert', false);
      await grid.getByRole('button', { name: /^Formal / }).click();
      await expect.poll(() => session.invoke('get_setting', { key: 'default_tone' })).toBe('formal');
      expect(await session.invoke('get_setting', { key: 'cleanup_intensity' })).toBe(intensity);
      await page.reload();
      await page.locator('[data-debug-id="nav.style"]').click();
      if (await toneTab.isVisible()) await toneTab.click();
      await expect(grid.getByRole('button', { name: /^Formal / })).toHaveAttribute('aria-pressed', 'true');
      const savedContext = (await session.invoke('get_contexts')).find(row => row.id === context.id);
      expect(savedContext.tone ?? null).toBeNull();
      expect(savedContext.cleanup_intensity).toBe('light');
    } finally {
      await session.invoke('delete_context', { contextId: context.id });
      await session.invoke('save_setting', { key: 'cleanup_intensity', value: previousIntensity ?? 'medium' });
      await session.invoke('save_setting', { key: 'default_tone', value: previousTone ?? 'casual' });
    }
  });
}
