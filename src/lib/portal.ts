export function portal(node: HTMLElement) {
  const target = document.body;
  const placeholder = document.createComment('portal');
  node.parentNode?.insertBefore(placeholder, node);
  target.appendChild(node);

  return {
    destroy() {
      node.remove();
      placeholder.remove();
    },
  };
}
