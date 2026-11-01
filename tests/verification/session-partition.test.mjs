import test from 'node:test';
import assert from 'node:assert/strict';
import { sessionPartition, partitionFiles, taskSessionPartitions } from '../../scripts/verification/session-partition.mjs';

test('default owned verification retains every node file and both browser projects', () => {
  assert.deepEqual(sessionPartition([]), { suite: 'all', index: 1, count: 1, project: 'all' });
  assert.deepEqual(partitionFiles(['two.test.mjs', 'one.test.mjs'], sessionPartition([])), ['one.test.mjs', 'two.test.mjs']);
});

test('partitions are deterministic, disjoint and cover newly added files automatically', () => {
  for (const length of [6, 27, 28, 100]) {
    const files = Array.from({ length }, (_, index) => `test-${index}.test.mjs`);
    const selected = Array.from({ length: 6 }, (_, index) => partitionFiles(files, sessionPartition(['--suite', 'node', '--shard', `${index + 1}/6`])));
    assert.deepEqual(selected.flat().sort(), [...files].sort());
    assert.equal(new Set(selected.flat()).size, files.length);
    selected.forEach((partition, index) => assert.deepEqual(partition, partitionFiles([...files].reverse(), { index: index + 1, count: 6 })));
    assert.ok(Math.max(...selected.map(partition => partition.length)) - Math.min(...selected.map(partition => partition.length)) <= 1);
  }
});

test('task verification runs all six Node shards and both browser projects', () => {
  const partitions = taskSessionPartitions();
  const nodes = partitions.filter(partition => partition.suite === 'node');
  assert.deepEqual(nodes.map(({ index, count }) => [index, count]), [[1, 6], [2, 6], [3, 6], [4, 6], [5, 6], [6, 6]]);
  const files = Array.from({ length: 29 }, (_, index) => `file-${index}.test.mjs`);
  assert.deepEqual(nodes.flatMap(partition => partitionFiles(files, partition)).sort(), files.sort());
  const browsers = partitions.filter(partition => partition.suite === 'browser');
  assert.deepEqual(browsers.map(({ project, index, count }) => [project, index, count]), [
    ['desktop', 1, 4], ['desktop', 2, 4], ['desktop', 3, 4], ['desktop', 4, 4],
    ['phone', 1, 4], ['phone', 2, 4], ['phone', 3, 4], ['phone', 4, 4],
  ]);
});

test('invalid or incompatible selectors fail before any backend is launched', () => {
  for (const args of [
    ['--suite'], ['--suite', 'missing'], ['--suite', 'node', '--suite', 'browser'],
    ['--shard', '1/6'], ['--shard', '0/1'], ['--shard', '2/1'],
    ['--suite', 'node', '--shard', '1/65'], ['--suite', 'node', '--shard', 'NaN/6'],
    ['--project', 'phone'], ['--suite', 'browser', '--project', 'missing'],
    ['--suite', 'node', '--project', 'phone'], ['--suite', 'browser', '--shard', '1/4'],
  ]) assert.throws(() => sessionPartition(args), undefined, JSON.stringify(args));
  assert.throws(() => partitionFiles(['one.test.mjs'], { index: 2, count: 2 }), /no files/);
});

test('browser partitions accept an explicit project and shard', () => {
  assert.deepEqual(sessionPartition(['--suite', 'browser', '--project', 'phone', '--shard', '3/4']), { suite: 'browser', index: 3, count: 4, project: 'phone' });
});
