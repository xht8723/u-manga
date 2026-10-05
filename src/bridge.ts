import { invoke, convertFileSrc, isTauri } from '@tauri-apps/api/core';
import type { Commands } from './commands';
import { warningGroup } from './commit-warnings';
import { hosted, runtimeMode } from './runtime-mode';
export { hosted, runtimeMode };
export const native = isTauri();
let warningGeneration = 0;
const warningOwners = new WeakMap<object, () => boolean>();
/** A late receipt from a destroyed app may not publish into its replacement. */
export function beginWarningSession() {
  const generation = ++warningGeneration;
  return () => { if (warningGeneration === generation) warningGeneration++; };
}
export function call<K extends keyof Commands>(
  command: K,
  ...parameters: Commands[K]['args'] extends undefined
    ? [args?: undefined]
    : [args: Commands[K]['args']]
): Promise<Commands[K]['result']>;
export async function call(
  command: keyof Commands,
  args: Record<string, unknown> = {},
): Promise<unknown> {
  const current = warningOwners.get(args);
  warningOwners.delete(args);
  return execute(command, args, current);
}
/** Frontend ownership affects warning presentation only, never command execution/results. */
export function scopedCommands(capture: () => () => boolean): typeof call {
  return ((command: keyof Commands, args: Record<string, unknown> = {}) => {
    const owned = { ...args };
    warningOwners.set(owned, capture());
    return (call as (command: keyof Commands, args: Record<string, unknown>) => Promise<unknown>)(command, owned);
  }) as typeof call;
}
async function execute(command: keyof Commands, args: Record<string, unknown>, current = () => true): Promise<unknown> {
  const generation = warningGeneration;
  const result = await (native
    ? invoke<unknown>(command, args)
    : hosted ? (await import('./hosted')).hostedCall(command, args || {})
    : ((await import('./preview')).previewCall(command, args || {}) as Promise<unknown>));
  if (
    ['secret_set', 'model_verify', 'model_download', 'book_update', 'library_relocate'].includes(
      command,
    )
  )
    window.dispatchEvent(new Event('umanga-requirements-changed'));
  if (generation === warningGeneration && current() && result && typeof result === 'object' && 'warnings' in result && Array.isArray(result.warnings)) {
    const message = warningGroup(result.warnings);
    if (message) window.dispatchEvent(new CustomEvent('umanga-commit-warning', { detail: message }));
  }
  return result;
}
export const imageUrl = (path: string) =>
  native && !path.startsWith('data:image/') ? convertFileSrc(path) : path;
