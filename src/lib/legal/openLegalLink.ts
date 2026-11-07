/** Opens a legal document in the system browser, falling back to a new tab outside Tauri. */
export async function openLegalLink(url: string): Promise<void> {
  try {
    const { open } = await import('@tauri-apps/plugin-shell');
    await open(url);
  } catch {
    window.open(url, '_blank', 'noopener,noreferrer');
  }
}
