import { describe, expect, it } from 'vitest';
import { emptyGlossary } from './glossary';
import { GlossaryJobChanges, syncGlossary } from './glossary-sync';
import type { BookGlossary, JobsBatch } from './types';

const original = (): BookGlossary => ({
  ...emptyGlossary(),
  enabled: true,
  autoDetect: true,
  revision: 1,
  entries: [
    { source: 'アリス', target: '爱丽丝' },
    { source: '城', target: '城堡' },
  ],
  automaticSources: ['アリス'],
});
const learned = (base: BookGlossary): BookGlossary => ({
  ...structuredClone(base),
  revision: base.revision + 1,
  entries: [...structuredClone(base.entries), { source: '魔王', target: '魔王' }],
  automaticSources: [...base.automaticSources, '魔王'],
});
describe('live glossary reconciliation', () => {
  it('refreshes a clean view without creating an unsaved draft', () => {
    const base = original(),
      saved = learned(base),
      result = syncGlossary(base, base, saved)!;
    expect(result.conflict).toBe(false);
    expect(result.baseline).toEqual(saved);
    expect(result.draft).toEqual(saved);
    expect(result.draft).not.toBe(result.baseline);
  });
  it('rebases automatic additions while preserving edits, deletions, switches and unfinished rows', () => {
    const base = original(),
      draft = structuredClone(base),
      saved = learned(base);
    draft.entries[0].target = 'My translation';
    draft.entries.splice(1, 1);
    draft.entries.push({ source: '', target: '' });
    draft.enabled = false;
    const result = syncGlossary(base, draft, saved)!;
    expect(result.conflict).toBe(false);
    expect(result.appended).toBe(1);
    expect(result.baseline.revision).toBe(saved.revision);
    expect(result.draft.entries).toEqual([...draft.entries, saved.entries[2]]);
    expect(result.draft.enabled).toBe(false);
    expect(result.draft.automaticSources).toContain('魔王');
    expect(draft.entries).toHaveLength(2);
    const next = {
      ...learned(saved),
      revision: 3,
      entries: [...saved.entries, { source: '勇者', target: '勇者' }],
      automaticSources: [...saved.automaticSources, '勇者'],
    };
    expect(syncGlossary(result.baseline, result.draft, next)?.draft.entries.at(-1)?.source).toBe(
      '勇者',
    );
  });
  it('never rebases over conflicting local terms or remote manual edits', () => {
    const base = original(),
      draft = structuredClone(base),
      saved = learned(base);
    draft.entries.push({ source: ' 魔王 ', target: 'My term' });
    const result = syncGlossary(base, draft, saved)!;
    expect(result.conflict).toBe(true);
    expect(result.baseline.revision).toBe(1);
    expect(result.draft).toEqual(draft);
    draft.entries.pop();
    draft.enabled = false;
    for (const remote of [
      { ...saved, enabled: false },
      { ...saved, entries: [{ source: 'アリス', target: 'Changed' }, ...saved.entries.slice(1)] },
      { ...saved, automaticSources: ['魔王'] },
      { ...saved, automaticSources: base.automaticSources },
    ])
      expect(syncGlossary(base, draft, remote)?.conflict).toBe(true);
  });
  it('ignores late snapshots and refreshes safely after the user discards conflicting changes', () => {
    const base = original(),
      saved = learned(base);
    expect(syncGlossary(saved, saved, base)).toBeNull();
    expect(syncGlossary(base, base, base)).toBeNull();
    const conflict = structuredClone(base);
    conflict.entries[0].target = 'Draft';
    const remote = structuredClone(saved);
    remote.entries[0].target = 'Remote';
    expect(syncGlossary(base, conflict, remote)?.conflict).toBe(true);
    expect(syncGlossary(base, base, remote)?.draft).toEqual(remote);
  });
});
describe('glossary job invalidation', () => {
  const batch = (id: string, added: number, generation = 1, project = 'book') =>
    ({
      generation,
      jobs: [{ id, project, glossaryCheckpoint: { added } }],
    }) as JobsBatch;
  it('refreshes only durable additions, coalescing per book and ignoring stale/timer events', () => {
    const tracker = new GlossaryJobChanges();
    expect(tracker.apply(batch('one', 0))).toEqual([]);
    expect(tracker.apply(batch('one', 2))).toEqual(['book']);
    expect(tracker.apply(batch('one', 2))).toEqual([]);
    expect(tracker.apply(batch('one', 1))).toEqual([]);
    expect(
      tracker.apply({
        generation: 1,
        jobs: [...batch('one', 3).jobs, ...batch('two', 1).jobs],
      } as JobsBatch),
    ).toEqual(['book']);
    expect(tracker.apply(batch('three', 1, 1, 'other'))).toEqual(['other']);
    expect(tracker.apply(batch('one', 1, 2))).toEqual(['book']);
    expect(tracker.apply(batch('one', 9, 1))).toEqual([]);
  });
});
