// Files stay serial within a runner; CI isolates partitions on separate machines.
export function sessionPartition(args) {
  const value = (flag, fallback) => {
    const index = args.indexOf(flag);
    if (index < 0) return fallback;
    if (args.lastIndexOf(flag) !== index || !args[index + 1] || args[index + 1].startsWith('--')) {
      throw new Error(`${flag} requires exactly one value`);
    }
    return args[index + 1];
  };
  const suite = value('--suite', 'all');
  const shard = value('--shard', '1/1');
  const project = value('--project', 'all');
  if (!['all', 'node', 'browser'].includes(suite)) throw new Error('Unknown session suite');
  if (!/^[1-9]\d*\/[1-9]\d*$/.test(shard)) throw new Error('Expected --shard index/count');
  const [index, count] = shard.split('/').map(Number);
  if (!Number.isSafeInteger(index) || !Number.isSafeInteger(count) || index > count || count > 64) throw new Error('Invalid session shard');
  if (suite !== 'node' && shard !== '1/1') throw new Error('--shard requires --suite node');
  if (!['all', 'desktop', 'phone'].includes(project) || (suite !== 'browser' && project !== 'all')) throw new Error('--project requires --suite browser and a known project');
  return { suite, index, count, project };
}

export function partitionFiles(files, { index, count }) {
  const selected = [...files].sort().filter((_, offset) => offset % count === index - 1);
  if (!selected.length) throw new Error('Owned-session shard contains no files');
  return selected;
}
