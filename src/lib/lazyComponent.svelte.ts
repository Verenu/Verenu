import type { Component } from 'svelte';

export function lazyComponent<C extends Component>(importComponent: () => Promise<{ default: C }>) {
  let component = $state.raw<C | null>(null);
  let error = $state(false);
  let pending: Promise<void> | undefined;

  return {
    get component() { return component; },
    get error() { return error; },
    load(): Promise<void> {
      if (component) return Promise.resolve();
      if (pending) return pending;
      error = false;
      pending = importComponent().then((module) => {
        component = module.default;
      }).catch(() => {
        error = true;
      }).finally(() => {
        pending = undefined;
      });
      return pending;
    },
  };
}

export type LazyComponent = ReturnType<typeof lazyComponent>;
