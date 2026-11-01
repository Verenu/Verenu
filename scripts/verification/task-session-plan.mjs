export function taskSessionPartitions() {
  return [
    ...Array.from({ length: 8 }, (_, index) => ({ suite: 'node', index: index + 1, count: 8, project: 'all' })),
    { suite: 'browser', index: 1, count: 1, project: 'desktop' },
    { suite: 'browser', index: 1, count: 1, project: 'phone' },
  ];
}

export function taskSessionArguments(partition) {
  return ['--suite', partition.suite, '--shard', `${partition.index}/${partition.count}`,
    ...(partition.suite === 'browser' ? ['--project', partition.project] : [])];
}

export function matchesTaskSessionPartition(actual, expected) {
  return actual?.suite === expected.suite && actual?.index === expected.index
    && actual?.count === expected.count && actual?.project === expected.project;
}

export function matchesTaskSessionMode(report, category) {
  return report.live === (category === 'pipeline');
}

export function taskSessionCoverage(records) {
  const expected = taskSessionPartitions();
  return records.length === expected.length && expected.every(partition =>
    records.filter(record => matchesTaskSessionPartition(record.partition, partition)).length === 1);
}
