export type ScrollEdgeCallback = (top: boolean, bottom: boolean, node: HTMLElement) => void;

export function scrollEdges(node: HTMLElement, onChange: ScrollEdgeCallback) {
  let callback = onChange;
  const update = () => {
    const { scrollTop, scrollHeight, clientHeight } = node;
    callback(scrollTop > 4, scrollTop + clientHeight < scrollHeight - 4, node);
  };
  update();
  node.addEventListener('scroll', update, { passive: true });
  const observer = new ResizeObserver(update);
  observer.observe(node);
  return {
    update(next: ScrollEdgeCallback) {
      callback = next;
      update();
    },
    destroy() {
      node.removeEventListener('scroll', update);
      observer.disconnect();
    },
  };
}
