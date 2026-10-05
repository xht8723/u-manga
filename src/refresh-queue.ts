/** Collapse bursts while replaying invalidations received during an outstanding read. */
export function refreshQueue<K>(refresh: (key: K) => Promise<void>) {
  const active = new Map<K, { dirty: boolean; result: Promise<void> }>();
  return (key: K): Promise<void> => {
    const existing = active.get(key);
    if (existing) {
      existing.dirty = true;
      return existing.result;
    }
    const entry = { dirty: true, result: Promise.resolve() };
    active.set(key, entry);
    entry.result = (async () => {
      try {
        do {
          entry.dirty = false;
          await refresh(key);
        } while (entry.dirty);
      } finally {
        active.delete(key);
      }
    })();
    return entry.result;
  };
}
