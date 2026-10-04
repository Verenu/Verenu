import assert from 'node:assert/strict';

const pause = ms => new Promise(resolve => setTimeout(resolve, ms));

// Uses the actual Svelte overlay and production native window commands.
// OS pointer routing is checked separately against a disposable desktop target.
export async function verifyNativePill({ browser, invoke, directory, screenshot }) {
  const show = async (state, message, context) => {
    await browser.command('POST', '/window', { handle: 'main' });
    await invoke('native_test_pill', { state, message, context });
    await browser.command('POST', '/window', { handle: 'pill' });
    await pause(1000);
  };
  const layout = () => browser.execute(`
    const pill = document.querySelector('.pill');
    const cluster = document.querySelector('.pill-cluster');
    return { viewport: [innerWidth, innerHeight], capsule: pill ? pill.getBoundingClientRect().toJSON() : null,
      cluster: cluster ? [cluster.offsetWidth, cluster.offsetHeight] : null, controls: [...document.querySelectorAll('.pill button')].map(b => b.getAttribute('aria-label')) };
  `);
  const assertFit = box => {
    assert.ok(box.capsule, 'Visible native pill is missing');
    assert.ok(box.cluster, 'Native pill cluster is missing');
    const { left, top, right, bottom } = box.capsule;
    assert.ok(left >= 0 && top >= 0 && right <= box.viewport[0] && bottom <= box.viewport[1], 'Native capsule is clipped');
    assert.ok(box.viewport[0] >= box.cluster[0] && box.viewport[0] <= box.cluster[0] + 28, 'Native window keeps excessive horizontal margin');
    assert.ok(Math.abs(box.viewport[1] - box.cluster[1] - 20) <= 1, 'Native window keeps excessive vertical margin');
  };
  const assertInput = async (state, enabled) => {
    const policy = await invoke('native_test_pill');
    assert.equal(policy.state, state);
    assert.equal(policy.interactive, enabled, `Incorrect native input policy for ${state}`);
    if (enabled) {
      const box = (await layout()).capsule;
      assert.ok(box, 'Interactive native pill is missing');
      assert.ok(policy.rect, 'Interactive capsule has no input rectangle');
      for (const [actual, expected] of policy.rect.map((value, i) => [value, [box.x, box.y, box.width, box.height][i]])) {
        assert.ok(Math.abs(actual - expected) <= 1.5, 'Native input rectangle does not follow the capsule');
      }
    }
  };
  try {
    await show('handsfree', null, 'Everywhere');
    const handsfree = await layout(); assertFit(handsfree); await assertInput('handsfree', true);
    assert.deepEqual(handsfree.controls, ['Cancel', 'Confirm']);
    await screenshot(`${directory}/pill-handsfree.png`);
    await invoke('set_pill_interactive', { interactive: false, expectedState: 'recording' });
    await assertInput('handsfree', true);

    for (const state of ['recording', 'processing', 'loading_local_model']) {
      await show(state, null, 'Everywhere'); assertFit(await layout());
      await invoke('set_pill_interactive', { interactive: true });
      await assertInput(state, false);
    }
    await screenshot(`${directory}/pill-passive-loading.png`);

    await show('error', 'Synthetic connection error. Check your connection and try again. '.repeat(5));
    const error = await layout(); assertFit(error); await assertInput('error', true);
    assert.ok(error.viewport[0] > handsfree.viewport[0] && error.viewport[1] > handsfree.viewport[1], 'Error window did not grow to fit wrapped text');
    await screenshot(`${directory}/pill-error.png`);

    for (const state of ['cancelled', 'interrupted', 'paste_failed', 'copied', 'clipboard_warning']) {
      await show(state, 'Synthetic status'); assertFit(await layout()); await assertInput(state, true);
    }
    for (let i = 0; i < 3; i++) {
      await show('handsfree', null, 'Everywhere');
      await show('recording', null, 'Everywhere');
      await invoke('set_pill_interactive', { interactive: true, expectedState: 'handsfree' });
      await assertInput('recording', false);
      const recording = await layout(); assertFit(recording);
      assert.ok(recording.viewport[0] < handsfree.viewport[0], 'Recording did not return to compact bounds');
    }
    await screenshot(`${directory}/pill-recording.png`);
    return ['pill-handsfree.png', 'pill-passive-loading.png', 'pill-error.png', 'pill-recording.png'].map(name => `${directory}/${name}`);
  } finally {
    await browser.command('POST', '/window', { handle: 'main' });
    await invoke('native_test_pill', { state: 'idle' });
  }
}
