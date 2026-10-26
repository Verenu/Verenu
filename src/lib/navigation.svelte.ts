import { untrack } from 'svelte';
import type { LazyComponent } from './lazyComponent.svelte';

/** Keep the current view until the latest requested module can render. */
export function navigation<T extends string>(
  requested: () => T,
  viewFor: (selection: T) => LazyComponent | null,
) {
  let current = $state(untrack(requested));
  let loading = $state(false);

  $effect(() => {
    const next = requested();
    const view = viewFor(next);
    let active = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    untrack(() => {
      loading = false;
      if (!view || view.component || view.error) {
        current = next;
      } else {
        timer = setTimeout(() => { loading = true; }, 150);
        void view.load().then(() => {
          if (!active) return;
          clearTimeout(timer);
          loading = false;
          current = next;
        });
      }
    });
    return () => {
      active = false;
      clearTimeout(timer);
    };
  });

  return {
    get current() { return current; },
    get pending() { return requested() !== current; },
    get loading() { return loading; },
  };
}
