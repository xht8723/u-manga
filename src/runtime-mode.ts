import { isTauri } from '@tauri-apps/api/core';
/** A hosted failure must never open the demo library. Only the server supplies this marker. */
export const runtimeMode: 'desktop' | 'hosted' | 'preview' = isTauri()
  ? 'desktop'
  : typeof document !== 'undefined' && document.querySelector('meta[name="umanga-runtime"]')?.getAttribute('content') === 'hosted'
    ? 'hosted' : 'preview';
export const hosted = runtimeMode === 'hosted';
