import { listen } from '@tauri-apps/api/event';
import { native } from './bridge';
export type ContentChange = { path: string | null; pageId: string | null };
export async function listenContent(callback: (change: ContentChange) => void) {
  if (native) return listen<ContentChange>('content-changed', event => callback(event.payload));
  const handler = (event: Event) => callback((event as CustomEvent<ContentChange>).detail);
  window.addEventListener('umanga-content-changed', handler);
  return () => window.removeEventListener('umanga-content-changed', handler);
}
