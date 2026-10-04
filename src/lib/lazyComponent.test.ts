import { describe, expect, it, vi } from 'vitest';
import type { Component } from 'svelte';
import { lazyComponent } from './lazyComponent.svelte';

describe('deferred components', () => {
  it('loads only on demand, shares pending imports, and reuses the loaded component', async () => {
    const component: Component = () => ({});
    let resolve!: (value: { default: Component }) => void;
    const importer = vi.fn(() => new Promise<{ default: Component }>((done) => { resolve = done; }));
    const view = lazyComponent(importer);
    expect(importer).not.toHaveBeenCalled();
    const first = view.load();
    expect(view.load()).toBe(first);
    resolve({ default: component });
    await first;
    expect(view.component).toBe(component);
    await view.load();
    expect(importer).toHaveBeenCalledTimes(1);
  });

  it('allows a failed import to be retried without an unhandled rejection', async () => {
    const component: Component = () => ({});
    const importer = vi.fn<() => Promise<{ default: Component }>>()
      .mockRejectedValueOnce(new Error('offline'))
      .mockResolvedValueOnce({ default: component });
    const view = lazyComponent(importer);
    await view.load();
    expect(view.error).toBe(true);
    expect(view.component).toBeNull();
    await view.load();
    expect(view.error).toBe(false);
    expect(view.component).toBe(component);
  });
});
