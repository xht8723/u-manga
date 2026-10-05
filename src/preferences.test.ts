import { describe, it, expect } from 'vitest';
import { mergePreferences, preferenceCoordinator, checkedRelocation } from './preferences';
import { settingsDefaults } from './book-state';
import type { Settings } from './types';
describe('preference ownership', () => {
  it('retains unrelated concurrent intentions and publishes durable order', async () => {
    const base = settingsDefaults(); let disk = structuredClone(base), ui = structuredClone(base);
    const save = preferenceCoordinator(async (next, old) => {
      await new Promise(r => setTimeout(r, 5));
      disk = mergePreferences(old, next, disk); return structuredClone(disk);
    }, saved => { ui = saved; });
    const a = structuredClone(base); a.appearance.active = 'night';
    const b = structuredClone(base); b.libraryView = 'list';
    await Promise.all([save(a, base), save(b, base)]);
    expect(ui).toEqual(disk); expect(ui.appearance.active).toBe('night'); expect(ui.libraryView).toBe('list');
  });
  it('rejects conflicting edits and recovers after a failed write', async () => {
    const base = settingsDefaults(), a = structuredClone(base), b = structuredClone(base);
    a.concurrentBooks = 3; b.concurrentBooks = 4;
    expect(() => mergePreferences(base, b, a)).toThrow('Settings changed elsewhere');
    let fail = true, published = 0;
    const save = preferenceCoordinator(async next => { if (fail) { fail = false; throw Error('disk'); } return next; }, () => published++);
    await expect(save(a, base)).rejects.toThrow('disk'); expect(published).toBe(0);
    await save(b, base); expect(published).toBe(1);
  });
});

describe('library relocation ownership', () => {
  it('makes no native mutation after Keep editing or a failed preflight', async () => {
    const base = settingsDefaults(); let moves = 0, reads = 0;
    const relocate = async () => { moves++; return base; };
    const current = async () => { reads++; return base; };
    await expect(checkedRelocation('old', async () => false, relocate, current)).rejects.toThrow('cancelled');
    await expect(checkedRelocation('old', async () => { throw Error('save failed'); }, relocate, current)).rejects.toThrow('save failed');
    expect(moves).toBe(0); expect(reads).toBe(0);
  });
  it('finishes saving at the old root before moving and acknowledges committed warnings once', async () => {
    const base = settingsDefaults(); base.libraryDirectory = 'old';
    let disk = structuredClone(base), decisions = 0;
    const events: string[] = [], published: string[] = [];
    const coordinator = preferenceCoordinator(async next => next, saved => published.push(saved.libraryDirectory));
    let warning: unknown;
    await coordinator.perform(async () => {
      const result = await checkedRelocation('old', async () => {
        decisions++; events.push('saved at ' + disk.libraryDirectory); return true;
      }, async () => {
        events.push('move'); disk.libraryDirectory = 'new'; throw Error('retirement warning');
      }, async () => structuredClone(disk));
      warning = result.warning; return result.settings;
    });
    expect(events).toEqual(['saved at old', 'move']);
    expect(published).toEqual(['new']); expect(decisions).toBe(1);
    expect(String(warning)).toContain('retirement warning');
  });
  it('keeps external activation and queued preferences in durable publication order', async () => {
    const base = settingsDefaults(); base.libraryDirectory = 'old';
    let disk = structuredClone(base); const publications: Settings[] = [];
    let release!: () => void;
    const held = new Promise<void>(r => release = r);
    const save = preferenceCoordinator(async (next, old) => {
      disk = mergePreferences(old, next, disk); return structuredClone(disk);
    }, value => publications.push(value));
    const moving = save.perform(async () => {
      await held; disk.libraryDirectory = 'new'; return structuredClone(disk);
    });
    const night = structuredClone(base); night.appearance.active = 'night';
    const pending = save(night, base);
    release(); await Promise.all([moving, pending]);
    expect(publications.map(p => p.libraryDirectory)).toEqual(['new', 'new']);
    expect(publications.at(-1)?.appearance.active).toBe('night');
  });
});
