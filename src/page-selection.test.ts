import { describe, expect, it } from 'vitest';
import { crossesCard, PageSweep } from './page-selection';

describe('page sweep selection', () => {
  it('adds crossed cards once and preserves unrelated selection', () => {
    const sweep = new PageSweep(['other'], 'first');
    expect(sweep.visit(['second', 'first', 'second'])).toEqual(['other', 'first', 'second']);
    expect(sweep.before).toEqual(['other']);
  });
  it('starting on a selected card removes crossed cards without toggling unselected cards', () => {
    const sweep = new PageSweep(['first', 'second', 'other'], 'first');
    expect(sweep.visit(['second', 'third', 'first'])).toEqual(['other']);
    expect(sweep.before).toEqual(['first', 'second', 'other']);
  });
  it('hits cards skipped between events in both directions, excluding gaps', () => {
    const rect = { left: 20, right: 40, top: 20, bottom: 40 };
    expect(crossesCard({ x: 0, y: 30 }, { x: 100, y: 30 }, rect)).toBe(true);
    expect(crossesCard({ x: 100, y: 30 }, { x: 0, y: 30 }, rect)).toBe(true);
    expect(crossesCard({ x: 30, y: 30 }, { x: 30, y: 30 }, rect)).toBe(true);
    expect(crossesCard({ x: 0, y: 10 }, { x: 100, y: 10 }, rect)).toBe(false);
    expect(crossesCard({ x: 0, y: 0 }, { x: 19, y: 19 }, rect)).toBe(false);
  });
});
