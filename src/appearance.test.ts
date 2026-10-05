import { describe, it, expect } from 'vitest';
import { defaults, pageFilter } from './appearance';
describe('reading comfort boundaries', () => {
  it('starts with two independent neutral page profiles', () => {
    const a = defaults();
    expect(a.day.filters).toEqual(a.night.filters);
    a.night.filters.brightness = 0.5;
    expect(a.day.filters.brightness).toBe(1);
    expect(Object.keys(a).sort()).toEqual(['active', 'day', 'night']);
  });
  it('applies filters exclusively to reader', () => {
    const a = defaults();
    a.night.filters.inversion = 1;
    expect(pageFilter(a.night, 'Reader')).toContain('invert(1)');
    expect(pageFilter(a.night, 'Editor')).toBe('none');
    expect(pageFilter(a.night, 'Batch')).toBe('none');
  });
});
