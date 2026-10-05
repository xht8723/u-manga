import { afterEach, describe, expect, it, vi } from 'vitest';
import { latestCheck, type CheckState } from './latest-check';
import { readinessSignature, newProvider, ollamaDestination } from './setup-state';
import { settingsDefaults } from './book-state';
import { previewReadiness } from './setup-preview';

afterEach(() => vi.useRealTimers());
describe('stable readiness refreshes', () => {
  it('retains rows, immediately invalidates readiness, and hides fast activity flashes', async () => {
    vi.useFakeTimers();
    const states: CheckState<string>[] = [];
    const c = latestCheck(
      async (v: string) => v,
      (s) => states.push(s),
    );
    c.start('first');
    await vi.advanceTimersByTimeAsync(180);
    expect(states.at(-1)).toMatchObject({ value: 'first', fresh: true });
    c.start('second');
    expect(states.at(-1)).toMatchObject({ value: 'first', fresh: false, pending: true });
    await vi.advanceTimersByTimeAsync(180);
    expect(states.at(-1)).toMatchObject({ value: 'second', fresh: true });
    expect(states.some((s) => s.showActivity)).toBe(false);
    c.cancel();
  });
  it('ignores superseded successes/errors and retains previous rows on current failure', async () => {
    vi.useFakeTimers();
    const pending = new Map<
      string,
      { resolve: (v: string) => void; reject: (v: string) => void }
    >();
    let state: CheckState<string> | undefined;
    const c = latestCheck(
      (v: string) => new Promise<string>((resolve, reject) => pending.set(v, { resolve, reject })),
      (s) => {
        state = s;
      },
    );
    c.start('initial');
    await vi.advanceTimersByTimeAsync(180);
    pending.get('initial')!.resolve('saved');
    await Promise.resolve();
    await Promise.resolve();
    c.start('old');
    await vi.advanceTimersByTimeAsync(180);
    c.start('new');
    await vi.advanceTimersByTimeAsync(380);
    expect(state).toMatchObject({ value: 'saved', fresh: false, showActivity: true });
    pending.get('old')!.reject('outdated');
    await vi.advanceTimersByTimeAsync(1);
    expect(state!.error).toBe('');
    pending.get('new')!.reject('offline');
    await vi.advanceTimersByTimeAsync(1);
    expect(state).toMatchObject({ value: 'saved', fresh: false, pending: false, error: { fallback: 'offline', detail: 'offline' } });
    c.start('obsolete');
    await vi.advanceTimersByTimeAsync(180);
    c.start('current');
    await vi.advanceTimersByTimeAsync(180);
    pending.get('current')!.resolve('current');
    await vi.advanceTimersByTimeAsync(1);
    pending.get('obsolete')!.resolve('obsolete');
    await vi.advanceTimersByTimeAsync(1);
    expect(state!.value).toBe('current');
    c.cancel();
  });
  it('does not revalidate appearance or unrelated providers', () => {
    const s = settingsDefaults();
    const before = readinessSignature(s);
    s.appearance.active = 'night';
    s.providers.push(newProvider());
    expect(readinessSignature(s)).toBe(before);
    s.libraryDirectory = 'Models';
    expect(readinessSignature(s)).not.toBe(before);
  });
});
describe('Ollama setup', () => {
  it('identifies loopback addresses without treating similar network hostnames as local', () => {
    expect(ollamaDestination('http://127.0.0.1:11434')).toContain('This computer');
    expect(ollamaDestination('http://[::1]:11434')).toContain('This computer');
    expect(ollamaDestination('http://127.server.lan:11434')).toContain('Ollama server');
    expect(ollamaDestination('http://manga-pc:11434')).toContain('Ollama server');
  });
  it('uses discovered capabilities without a credential or manually asserted vision support', () => {
    const s = settingsDefaults(),
      p = newProvider();
    Object.assign(p, {
      protocol: 'ollama',
      endpoint: 'http://localhost:11434',
      model: 'local-vision:4b',
      vision: false,
    });
    s.providers = [p];
    s.translation.providerId = p.id;
    s.translation.mode = 'vision';
    s.translation.cleanup.method = 'solid';
    s.libraryDirectory = 'Fixture';
    expect(previewReadiness(s, [], [])).toMatchObject({
      ready: true,
      credentialRequired: false,
      credentialStored: false,
    });
    p.model = 'local-text:3b';
    expect(previewReadiness(s, [], []).ready).toBe(false);
    s.translation.mode = 'local';
    s.libraryDirectory = 'Models';
    expect(previewReadiness(s, [], ['Models/models:manga_ocr_onnx']).ready).toBe(true);
    p.model = 'remote:cloud';
    expect(previewReadiness(s, [], ['Models/models:manga_ocr_onnx']).ready).toBe(false);
    p.model = 'local-vision:4b';
    s.translation.mode = 'vision';
    p.endpoint = 'http://offline:11434';
    expect(previewReadiness(s, [], ['Models/models:manga_ocr_onnx']).ready).toBe(false);
  });
});
