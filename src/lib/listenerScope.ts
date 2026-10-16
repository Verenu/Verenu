export function createListenerScope() {
  let disposed = false;
  const unlisteners: Array<() => void> = [];
  return {
    track(registration: Promise<() => void>): Promise<void> {
      return registration.then((unlisten) => {
        if (disposed) unlisten();
        else unlisteners.push(unlisten);
      });
    },
    dispose(): void {
      disposed = true;
      for (const unlisten of unlisteners.splice(0)) unlisten();
    },
  };
}
