import test from 'node:test';
import assert from 'node:assert/strict';
import { partitionFiles, sessionPartition } from '../../scripts/verification/session-partition.mjs';
import { matchesTaskSessionMode } from '../../scripts/verification/task-session-plan.mjs';

test('session evidence must identify whether live providers were used', () => {
  assert.equal(matchesTaskSessionMode({ live: false }, 'session'), true);
  assert.equal(matchesTaskSessionMode({ live: false }, 'pipeline'), false);
  assert.equal(matchesTaskSessionMode({ live: true }, 'pipeline'), true);
  assert.equal(matchesTaskSessionMode({}, 'session'), false);
});
import { matchesTaskSessionPartition, taskSessionArguments, taskSessionCoverage, taskSessionPartitions } from '../../scripts/verification/task-session-plan.mjs';

test('task verification covers every node file and both browser projects in bounded partitions', () => {
  const plan = taskSessionPartitions();
  const parsed = plan.map(partition => sessionPartition(taskSessionArguments(partition)));
  const files = Array.from({ length: 29 }, (_, index) => `file-${index}.test.mjs`);
  const node = parsed.filter(partition => partition.suite === 'node').flatMap(partition => partitionFiles(files, partition));
  assert.deepEqual(node.sort(), files.sort());
  assert.deepEqual(parsed.filter(partition => partition.suite === 'browser').map(partition => partition.project).sort(), ['desktop', 'phone']);
  parsed.forEach((partition, index) => assert.ok(matchesTaskSessionPartition(partition, plan[index])));
  assert.equal(matchesTaskSessionPartition(parsed[0], plan[1]), false);
  assert.equal(matchesTaskSessionPartition(undefined, plan[0]), false);
  const records = parsed.map(partition => ({ partition }));
  assert.equal(taskSessionCoverage(records), true);
  assert.equal(taskSessionCoverage(records.slice(1)), false);
  assert.equal(taskSessionCoverage([...records.slice(0, 3), records[0]]), false);
});
