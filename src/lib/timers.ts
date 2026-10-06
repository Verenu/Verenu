/** Replace named timeouts and release their ownership before callbacks run. */
export function createTimers<Key extends string>() {
  const pending = new Map<Key, ReturnType<typeof setTimeout>>();
  function clear(...keys: Key[]) {
    for (const key of keys) {
      const timer = pending.get(key);
      if (timer !== undefined) clearTimeout(timer);
      pending.delete(key);
    }
  }
  return {
    clear,
    has: (key: Key) => pending.has(key),
    set(key: Key, callback: () => void, delay: number) {
      clear(key);
      pending.set(key, setTimeout(() => {
        pending.delete(key);
        callback();
      }, delay));
    },
    clearAll() { clear(...pending.keys()); },
  };
}
