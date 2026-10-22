<script lang="ts">
  import { tick } from 'svelte';

  let {
    open = $bindable(false),
    closeSelector = '',
    restoreFocus = true,
    optionSelector = '.ui-dropdown-option',
    children,
  }: {
    open: boolean;
    closeSelector?: string;
    restoreFocus?: boolean;
    optionSelector?: string;
    children?: import('svelte').Snippet;
  } = $props();

  let trigger: HTMLElement | null = null;
  let root: HTMLElement | null = null;
  let wasOpen = false;

  $effect(() => {
    if (open) {
      trigger = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      root = trigger ? trigger.closest(closeSelector || '.ui-dropdown') : null;
    }

    if (wasOpen && !open) {
      requestAnimationFrame(() => {
        if (!restoreFocus || !trigger?.isConnected) return;
        const active = document.activeElement;
        const claimed = active instanceof HTMLElement && active !== document.body && !(root?.contains(active));
        if (!claimed) trigger.focus();
      });
    }
    wasOpen = open;

    if (!open) return;

    let disposed = false;
    tick().then(() => {
      if (disposed) return;
      window.addEventListener('click', handleOutsideClick);
      focusOptionOnOpen();
    });
    window.addEventListener('keydown', handleWindowKeydown);

    return () => {
      disposed = true;
      window.removeEventListener('click', handleOutsideClick);
      window.removeEventListener('keydown', handleWindowKeydown);
    };
  });

  function focusOptionOnOpen() {
    if (!root) return;
    const options = Array.from(root.querySelectorAll<HTMLElement>(optionSelector));
    if (options.length === 0) return;
    const selected = options.find((option) => option.getAttribute('aria-selected') === 'true');
    const target = selected ?? options[0];
    target.focus();
    setOptionTabindexes(options, target);
  }

  function setOptionTabindexes(options: HTMLElement[], active: HTMLElement | null) {
    for (const option of options) option.tabIndex = option === active ? 0 : -1;
  }

  function handleOutsideClick(e: MouseEvent) {
    const target = e.target instanceof Element
      ? e.target
      : (e.target as Node)?.parentElement;
    const targetRoot = target?.closest(closeSelector || '.ui-dropdown');
    if (target && (root ? root.contains(target) || targetRoot === root : targetRoot)) return;
    open = false;
  }

  function handleWindowKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      if (!open) return;
      e.stopPropagation();
      open = false;
      return;
    }

    if (!open || !root) return;
    const target = e.target instanceof Element ? e.target : null;
    const option = target?.closest<HTMLElement>(optionSelector);
    if (!option) return;
    const menuRoot = option.closest(closeSelector || '.ui-dropdown');
    if (menuRoot !== root) return;

    const options = Array.from(root.querySelectorAll<HTMLElement>(optionSelector));
    const index = options.indexOf(option);
    if (index === -1) return;

    switch (e.key) {
      case 'ArrowDown':
        e.preventDefault();
        setOptionTabindexes(options, options[(index + 1) % options.length]);
        options[(index + 1) % options.length].focus();
        break;
      case 'ArrowUp':
        e.preventDefault();
        setOptionTabindexes(options, options[(index - 1 + options.length) % options.length]);
        options[(index - 1 + options.length) % options.length].focus();
        break;
      case 'Home':
        e.preventDefault();
        setOptionTabindexes(options, options[0]);
        options[0].focus();
        break;
      case 'End':
        e.preventDefault();
        setOptionTabindexes(options, options[options.length - 1]);
        options[options.length - 1].focus();
        break;
    }
  }
</script>

{@render children?.()}
