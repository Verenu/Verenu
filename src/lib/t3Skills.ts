/** Match known desktop identities, including versioned Windows nightlies. */
export function isT3App(executable: string): boolean {
  const name = executable.split(/[\\/]/).pop()?.toLowerCase().replace(/\.(exe|app)$/, '') ?? '';
  return ['t3code', 't3-code', 't3 code', 't3 code (nightly)', 't3code-nightly', 't3-code-nightly', 'com.t3tools.t3code', 'com.t3tools.t3code.nightly'].includes(name)
    || /^t3-code-nightly-[0-9][0-9.-]*$/.test(name);
}
