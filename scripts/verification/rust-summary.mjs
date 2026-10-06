export function executedRustTests(output) {
  return [...output.matchAll(/test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed;/g)]
    .reduce((total, match) => total + Number(match[1]) + Number(match[2]), 0);
}
