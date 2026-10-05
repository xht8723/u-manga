import { describe, expect, it } from 'vitest';
import { mergePageEdits, latestPageJob } from './editor-state';
import type { Page, Job, Region } from './types';
const region = (id: string): Region => ({
  id,
  bbox: [0, 0, 100, 100],
  bubble: null,
  kind: 'text',
  score: 1,
  source: '',
  target: '',
  direction: 'auto',
  style: { font: null, size: null, color: '#000000', fill: '#ffffff', lineGap: 0.2, outlineEnabled: false, outlineWidthPercent: 8, outlineColor: '#ffffff' },
  allowFill: false,
  overlayOnly: false,
  prepared: false,
  review: null,
});
const page = (): Page => ({
  id: 'page',
  number: 0,
  name: 'p',
  source: { path: 'original', kind: 'image', entry: null, index: 0 },
  width: 100,
  height: 100,
  fingerprint: '',
  revision: 0,
  regions: [region('one')],
  rendered: null,
  background: null,
  cleanup: null,
  status: 'new',
  error: null,
});
describe('confirmed Editor drafts', () => {
  it('merges outline fields individually and preserves them through geometry edits and history', () => {
    const base = page(), draft = structuredClone(base), latest = structuredClone(base);
    draft.regions[0].style.outlineEnabled = true;
    draft.regions[0].style.outlineWidthPercent = 12;
    latest.regions[0].style.outlineColor = '#ff8800';
    latest.regions[0].target = 'new translation';
    const merged = mergePageEdits(base, draft, latest);
    expect(merged.regions[0].style).toMatchObject({outlineEnabled: true, outlineWidthPercent: 12, outlineColor: '#ff8800'});
    const geometry = structuredClone(merged);
    geometry.regions[0].bbox = [10, 10, 80, 80];
    expect(mergePageEdits(merged, geometry, merged).regions[0].style).toEqual(merged.regions[0].style);
    const undone = mergePageEdits(draft, base, merged);
    expect(undone.regions[0].style).toMatchObject({outlineEnabled: false, outlineWidthPercent: 8, outlineColor: '#ff8800'});
    expect(undone.regions[0].target).toBe('new translation');
    expect(mergePageEdits(base, draft, undone)).toEqual(merged);
  });
  it('merges only edited fields without losing newer recognition, regions, or cleanup', () => {
    const base = page(),
      draft = structuredClone(base),
      latest = structuredClone(base);
    draft.regions[0].target = 'manual';
    draft.regions[0].style.size = 24;
    latest.revision = 4;
    latest.regions[0].source = 'OCR';
    latest.regions[0].style.color = '#ff0000';
    latest.regions.push(region('new'));
    const merged = mergePageEdits(base, draft, latest);
    expect(merged.regions[0]).toMatchObject({
      source: 'OCR',
      target: 'manual',
      style: { size: 24, color: '#ff0000' },
    });
    expect(merged.regions).toHaveLength(2);
    expect(merged.revision).toBe(4);
    expect(base.regions[0].target).toBe('');
    expect(latest.regions[0].target).toBe('');
  });
  it('preserves deliberate deletions/additions and refuses to recreate a removed edited region', () => {
    const base = page(),
      draft = structuredClone(base);
    draft.regions = [region('manual')];
    expect(mergePageEdits(base, draft, base).regions.map((r) => r.id)).toEqual(['manual']);
    const edited = structuredClone(base);
    edited.regions[0].target = 'edit';
    expect(() => mergePageEdits(base, edited, { ...base, regions: [] })).toThrow('removed');
  });
  it('scopes job selection to book and page, including failed jobs and ongoing retries', () => {
    const failed = {
      id: 'failed',
      project: 'a',
      pageId: 'page',
      created: '2026-01-02',
      status: 'failed',
    } as Job;
    const other = { ...failed, id: 'other', project: 'b', status: 'running' } as Job;
    expect(latestPageJob([failed, other], 'a', 'page')?.id).toBe('failed');
    expect(
      latestPageJob(
        [failed, { ...failed, id: 'retry', status: 'paused', created: '2026-01-03' }],
        'a',
        'page',
      )?.id,
    ).toBe('retry');
    expect(latestPageJob([failed], 'a', 'different')).toBeNull();
  });
  it('restores a deleted region before its surviving neighbor without reordering new job regions', () => {
    const base = page();
    base.regions = [region('b')];
    const draft = { ...base, regions: [region('a'), region('b')] };
    const latest = { ...base, regions: [region('new'), region('b')] };
    expect(mergePageEdits(base, draft, latest).regions.map((r) => r.id)).toEqual(['new', 'a', 'b']);
    expect(() =>
      mergePageEdits(base, draft, { ...latest, regions: [region('a'), region('b')] }),
    ).toThrow('already exists');
  });
});
