import type { Settings } from './types';

/** Changes are captured at intent time; acknowledgments publish strictly in order. */
export function preferenceCoordinator(
  commit: (next: Settings, base: Settings) => Promise<Settings>,
  publish: (saved: Settings) => void,
) {
  let tail: Promise<unknown> = Promise.resolve();
  function perform(operation: () => Promise<Settings>) {
    const result = tail.then(async () => {
      const saved = await operation();
      publish(saved);
      return saved;
    });
    tail = result.catch(() => {});
    return result;
  }
  const save = (next: Settings, base: Settings) => {
    const intended = structuredClone(next), baseline = structuredClone(base);
    return perform(() => commit(intended, baseline));
  };
  return Object.assign(save, { perform });
}

/** Relocation can fail after durable activation; acknowledge that root once. */
export async function checkedRelocation(
  oldRoot: string,
  beforeLeave: () => Promise<boolean>,
  relocate: () => Promise<Settings>,
  current: () => Promise<Settings>,
): Promise<{ settings: Settings; warning?: unknown }> {
  if (!(await beforeLeave()))
    throw Error('Library change cancelled; finish the current page edits first.');
  try {
    return { settings: await relocate() };
  } catch (warning) {
    const settings = await current();
    if (settings.libraryDirectory === oldRoot) throw warning;
    return { settings, warning };
  }
}

/** Preview uses the same three-way field policy as native preferences. */
export function mergePreferences(base: Settings, next: Settings, current: Settings): Settings {
  function merge(b: any, n: any, c: any, path: string): any {
    if (JSON.stringify(b) === JSON.stringify(n)) return structuredClone(c);
    if (b && n && c && !Array.isArray(n) && typeof n === 'object') {
      return Object.fromEntries(Object.keys(n).map(k => [k, merge(b[k], n[k], c[k], `${path}.${k}`)]));
    }
    if (JSON.stringify(c) !== JSON.stringify(b) && JSON.stringify(c) !== JSON.stringify(n))
      throw Error(`Settings changed elsewhere: ${path}. Reopen Settings before applying.`);
    return structuredClone(n);
  }
  return merge(base, next, current, 'settings');
}
