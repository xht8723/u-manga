import { get, writable } from 'svelte/store';
import type { Provider } from './types';

// Match the native credential's provider + origin scope. No key enters this identity.
export function credentialIdentity(p: Provider) {
  const official: Record<string, string> = {
    google: 'https://translation.googleapis.com/',
    microsoft: 'https://api.cognitive.microsofttranslator.com/',
    deepl: 'https://api.deepl.com/',
    baidu: 'https://fanyi-api.baidu.com/',
  };
  const endpoint =
    official[p.service] ||
    p.endpoint.trim() ||
    (p.protocol === 'anthropic'
      ? 'https://api.anthropic.com/'
      : p.protocol === 'gemini'
        ? 'https://generativelanguage.googleapis.com/'
        : 'https://api.openai.com/');
  try {
    return `${p.id}:${new URL(endpoint).origin}`;
  } catch {
    return `${p.id}:${endpoint}`;
  }
}

export type CredentialStatus = {
  pending: boolean;
  error: string;
  saved: boolean;
  revision: number;
};
export function createCredentialAutosave(
  save: (profile: Provider, value: string) => Promise<unknown>,
  delay = 600,
) {
  type Entry = {
    next?: { profile: Provider; value: string };
    timer?: ReturnType<typeof setTimeout>;
    running?: Promise<void>;
    revision: number;
  };
  const entries = new Map<string, Entry>();
  const states = writable<Record<string, CredentialStatus>>({});
  function publish(id: string, e: Entry, error = '', saved = false) {
    states.update((s) => ({
      ...s,
      [id]: { pending: !!e.next || !!e.running, error, saved, revision: e.revision },
    }));
  }
  async function drain(id: string): Promise<void> {
    const e = entries.get(id);
    if (!e) return;
    clearTimeout(e.timer);
    if (e.running) {
      await e.running;
      return drain(id);
    }
    if (!e.next) return;
    const pending = e.next;
    const revision = e.revision;
    e.next = undefined;
    let error = '';
    e.running = (async () => {
      try {
        await save(pending.profile, pending.value);
      } catch {
        error = 'Could not save API key. Re-enter it to retry.';
      } finally {
        pending.value = '';
      }
    })();
    publish(id, e);
    await e.running;
    e.running = undefined;
    publish(id, e, revision === e.revision ? error : '', !error && !e.next);
    if (e.next) await drain(id);
  }
  function schedule(profile: Provider, value: string) {
    const id = credentialIdentity(profile);
    const e = entries.get(id) || { revision: 0 };
    clearTimeout(e.timer);
    e.revision++;
    e.next = value.trim() ? { profile: structuredClone(profile), value } : undefined;
    entries.set(id, e);
    publish(id, e);
    if (e.next) e.timer = setTimeout(() => void drain(id), delay);
  }
  async function flush(profile?: Provider) {
    const ids = profile ? [credentialIdentity(profile)] : [...entries.keys()];
    await Promise.all(ids.map(drain));
    const failed = ids.map((id) => get(states)[id]).find((s) => s?.error);
    if (failed) throw Error(failed.error);
  }
  return { states, schedule, flush };
}
