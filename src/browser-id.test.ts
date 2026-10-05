import { afterEach, describe, expect, it, vi } from 'vitest';
import { randomUuid } from './browser-id';

afterEach(() => vi.unstubAllGlobals());

describe('browser UUIDs on secure and LAN HTTP origins', () => {
  it('uses the native UUID method when available', () => {
    const randomUUID = vi.fn(() => '12345678-1234-4234-8234-123456789abc');
    vi.stubGlobal('crypto', { randomUUID });
    expect(randomUuid()).toBe('12345678-1234-4234-8234-123456789abc');
    expect(randomUUID).toHaveBeenCalledOnce();
  });

  it('encodes random bytes with v4 and RFC 4122 variant bits without randomUUID', () => {
    const getRandomValues = vi.fn((bytes: Uint8Array) => {
      bytes.set(Array.from({ length: 16 }, (_, i) => i));
      return bytes;
    });
    vi.stubGlobal('crypto', { getRandomValues });
    expect(randomUuid()).toBe('00010203-0405-4607-8809-0a0b0c0d0e0f');
    expect(getRandomValues).toHaveBeenCalledOnce();
  });

  it('retains fresh random bytes for each call', () => {
    let sequence = 0;
    vi.stubGlobal('crypto', { getRandomValues: (bytes: Uint8Array) => bytes.fill(++sequence) });
    const first = randomUuid(), second = randomUuid();
    expect(first).not.toBe(second);
    expect(first).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
  });

  it('fails clearly if secure randomness itself is unavailable', () => {
    vi.stubGlobal('crypto', undefined);
    expect(() => randomUuid()).toThrow('This browser cannot generate secure identifiers');
  });
});
