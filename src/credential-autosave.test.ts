import { afterEach, describe, expect, it, vi } from 'vitest';
import { get } from 'svelte/store';
import { createCredentialAutosave, credentialIdentity } from './credential-autosave';
import { newProvider } from './setup-state';

afterEach(() => vi.useRealTimers());
describe('credential autosave', () => {
  it('debounces typing, flushes on navigation, and exposes no secret in UI state', async () => {
    vi.useFakeTimers();
    const write = vi.fn().mockResolvedValue(undefined),
      p = newProvider();
    const c = createCredentialAutosave(write);
    c.schedule(p, 'first');
    c.schedule(p, 'replacement');
    expect(write).not.toHaveBeenCalled();
    expect(get(c.states)[credentialIdentity(p)].pending).toBe(true);
    await c.flush(p);
    expect(write).toHaveBeenCalledTimes(1);
    expect(write.mock.calls[0][1]).toBe('replacement');
    expect(JSON.stringify(get(c.states))).not.toContain('replacement');
    await vi.advanceTimersByTimeAsync(1000);
    expect(write).toHaveBeenCalledTimes(1);
    expect(get(c.states)[credentialIdentity(p)]).toMatchObject({ saved: true, pending: false });
  });
  it('captures the endpoint at input time and does not save a key to a newly selected host', async () => {
    const write = vi.fn().mockResolvedValue(undefined),
      p = newProvider();
    p.endpoint = 'https://original.invalid/v1';
    const c = createCredentialAutosave(write);
    c.schedule(p, 'test-secret');
    p.endpoint = 'https://different.invalid/v1';
    await c.flush();
    expect(write.mock.calls[0][0].endpoint).toBe('https://original.invalid/v1');
    expect(get(c.states)[credentialIdentity(p)]).toBeUndefined();
  });
  it('serializes active writes so the most recent value wins for the same origin', async () => {
    let finish!: () => void;
    const write = vi
      .fn()
      .mockImplementationOnce(() => new Promise<void>((r) => (finish = r)))
      .mockResolvedValue(undefined);
    const c = createCredentialAutosave(write),
      p = newProvider();
    p.endpoint = 'https://same.invalid/v1';
    c.schedule(p, 'old');
    const flushing = c.flush(p);
    p.endpoint = 'https://same.invalid/another-path';
    c.schedule(p, 'new');
    const second = c.flush(p);
    expect(write).toHaveBeenCalledTimes(1);
    finish();
    await Promise.all([flushing, second]);
    expect(write.mock.calls.map((c) => c[1])).toEqual(['old', 'new']);
    expect(get(c.states)[credentialIdentity(p)].pending).toBe(false);
  });
  it('reports failed saves, blocks navigation flush, and clears the error after correction', async () => {
    const write = vi
      .fn()
      .mockRejectedValueOnce(Error('OS store unavailable'))
      .mockResolvedValue(undefined);
    const c = createCredentialAutosave(write),
      p = newProvider();
    c.schedule(p, 'first');
    await expect(c.flush(p)).rejects.toThrow('Could not save API key');
    expect(get(c.states)[credentialIdentity(p)].saved).toBe(false);
    c.schedule(p, 'corrected');
    await c.flush(p);
    expect(get(c.states)[credentialIdentity(p)]).toMatchObject({ error: '', saved: true });
  });
  it('does not save blank input or remove an existing credential', async () => {
    const write = vi.fn(),
      c = createCredentialAutosave(write),
      p = newProvider();
    c.schedule(p, 'key');
    c.schedule(p, '');
    await c.flush();
    expect(write).not.toHaveBeenCalled();
  });
});
