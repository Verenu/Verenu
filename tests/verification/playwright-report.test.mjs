import test from 'node:test';
import assert from 'node:assert/strict';
import { summarizePlaywrightFailures } from '../../scripts/verification/playwright-report.mjs';
import { summarizePlaywrightReport } from '../../scripts/verification/playwright-summary.mjs';

test('Playwright summaries retain failed desktop and phone case names without assertion data', () => {
  const report = {
    suites: [{
      file: 'tests/browser/api-keys.spec.mjs',
      specs: [{
        title: 'API key actions never expose the inactive label',
        line: 3,
        tests: [
          { projectName: 'desktop', results: [{ status: 'failed', errors: [{ message: 'private assertion value' }] }] },
          { projectName: 'phone', results: [{ status: 'timedOut', errors: [{ message: 'private assertion value' }] }] },
          { projectName: 'desktop', results: [{ status: 'passed' }] },
        ],
      }],
      suites: [{
        file: 'tests/browser/models.spec.mjs',
        specs: [{
          title: 'Model state survives reload',
          line: 19,
          tests: [{ projectName: 'desktop', results: [{ status: 'passed' }, { status: 'failed' }] }],
        }],
      }],
    }],
  };

  assert.deepEqual(summarizePlaywrightFailures(report).map(({ diagnostics, ...metadata }) => metadata), [
    { project: 'desktop', file: 'api-keys.spec.mjs', line: 3, title: 'API key actions never expose the inactive label' },
    { project: 'desktop', file: 'models.spec.mjs', line: 19, title: 'Model state survives reload' },
    { project: 'phone', file: 'api-keys.spec.mjs', line: 3, title: 'API key actions never expose the inactive label' },
  ]);
});

test('Playwright summaries omit passing, skipped, and missing results', () => {
  assert.deepEqual(summarizePlaywrightFailures({
    suites: [{ specs: [{ title: 'Passing test', tests: [{ results: [{ status: 'passed' }] }] }] }],
  }), []);
  assert.deepEqual(summarizePlaywrightFailures(null), []);
});

test('Playwright summaries retain only allowlisted menu geometry diagnostics', () => {
  const report = {
    suites: [{
      file: 'tests/browser/settings-dropdowns.spec.mjs',
      specs: [{
        title: 'menus stay in the viewport',
        line: 34,
        tests: [{
          projectName: 'phone',
          results: [{
            status: 'failed',
            errors: [{
              message: 'private assertion value\nMENU_GEOMETRY:{"menuId":"transcription-mode-menu","check":"right-panel","bounds":{"x":8,"y":12,"width":374,"height":240,"privateText":"do-not-share"},"content":{"x":16,"y":24,"width":358,"height":700},"viewport":{"width":390,"height":844},"privateText":"do-not-share"}',
              location: { file: '/runner/work/app/tests/browser/settings-dropdowns.spec.mjs', line: 80, column: 5 },
            }],
          }],
        }],
      }],
    }],
  };

  assert.deepEqual(summarizePlaywrightFailures(report).map(({ diagnostics, ...metadata }) => metadata), [{
    project: 'phone',
    file: 'settings-dropdowns.spec.mjs',
    line: 34,
    title: 'menus stay in the viewport',
    assertionLine: 80,
    menuGeometry: {
      check: 'right-panel',
      menuId: 'transcription-mode-menu',
      bounds: { x: 8, y: 12, width: 374, height: 240 },
      content: { x: 16, y: 24, width: 358, height: 700 },
      viewport: { width: 390, height: 844 },
    },
  }]);

  const unsafe = {
    suites: [{
      specs: [{
        tests: [{
          projectName: 'phone',
          results: [{
            status: 'failed',
            errors: [{
              message: 'MENU_GEOMETRY:{"menuId":"private-menu","check":"right-panel","bounds":{"x":0,"y":0,"width":1,"height":1},"content":{"x":0,"y":0,"width":1,"height":1},"viewport":{"width":1,"height":1}}',
            }],
          }],
        }],
      }],
    }],
  };
  assert.equal(Object.hasOwn(summarizePlaywrightFailures(unsafe)[0], 'menuGeometry'), false);
});

test('flaky status stays failed while the task summary retains safe diagnostics from the failed attempt', () => {
  const report = {
    suites: [{
      file: 'tests/browser/settings-dropdowns.spec.mjs',
      specs: [{
        title: 'menus stay in the viewport',
        line: 34,
        tests: [{
          projectName: 'phone',
          status: 'flaky',
          results: [{
            status: 'failed',
            errors: [{
              message: 'MENU_GEOMETRY:{"menuId":"transcription-mode-menu","check":"right-panel","bounds":{"x":8,"y":12,"width":374,"height":240},"content":{"x":16,"y":24,"width":358,"height":700},"viewport":{"width":390,"height":844}}',
              location: { file: '/runner/work/app/tests/browser/settings-dropdowns.spec.mjs', line: 80 },
            }],
          }, { status: 'passed', retry: 1 }],
        }],
      }],
    }],
  };

  const masterSummary = summarizePlaywrightReport(report, { projectRoot: '/repo' });
  const taskFailures = summarizePlaywrightFailures(report);
  assert.equal(masterSummary.status, 'failed');
  assert.equal(masterSummary.tests[0].status, 'failed');
  assert.equal(masterSummary.tests[0].retryCount, 1);
  assert.deepEqual(taskFailures.map(({ diagnostics, ...metadata }) => metadata), [{
    project: 'phone',
    file: 'settings-dropdowns.spec.mjs',
    line: 34,
    title: 'menus stay in the viewport',
    assertionLine: 80,
    menuGeometry: {
      check: 'right-panel',
      menuId: 'transcription-mode-menu',
      bounds: { x: 8, y: 12, width: 374, height: 240 },
      content: { x: 16, y: 24, width: 358, height: 700 },
      viewport: { width: 390, height: 844 },
    },
  }]);
});
