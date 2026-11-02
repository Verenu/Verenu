export function taskSessionPartitions() {
  return [
    ...Array.from({ length: 8 }, (_, index) => ({ suite: 'node', index: index + 1, count: 8, project: 'all' })),
    ...['desktop', 'phone'].flatMap(project =>
      Array.from({ length: 4 }, (_, index) => ({ suite: 'browser', index: index + 1, count: 4, project }))),
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
