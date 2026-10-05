import { describe, it, expect } from 'vitest';
import { changedRegions, hasDraftChanges, scopedDraft, remainingDraft, acknowledgeDraft } from './editor-drafts';
import { mergePageEdits } from './editor-state';
import type { Page, Region } from './types';
const region = (id: string): Region => ({
  id,
  source: 'source',
  target: 'saved',
  bbox: [0, 0, 40, 100],
  bubble: null,
  kind: 'text',
  score: 1,
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
  name: 'page',
  source: { path: 'original', kind: 'image', entry: null, index: 0 },
  width: 200,
  height: 200,
  fingerprint: 'pixels',
  revision: 1,
  regions: [region('a'), region('b')],
  rendered: 'saved.png',
  background: null,
  cleanup: null,
  status: 'edited',
  error: null,
});
describe('region draft scopes', () => {
  it('saves A without committing B or a page background; rebases the remaining drafts', () => {
    const base = page(),
      draft = structuredClone(base);
    draft.regions[0].target = 'A edit';
    draft.regions[1].source = 'B edit';
    draft.background = 'pending.png';
    const sent = scopedDraft(base, draft, { regionId: 'a' });
    expect(sent.regions[1]).toEqual(base.regions[1]);
    expect(sent.background).toBeNull();
    const saved = { ...sent, revision: 2, rendered: 'new.png' };
    const pending = remainingDraft(base, draft, saved, { regionId: 'a' });
    expect(changedRegions(saved, pending)).toEqual(['b']);
    expect(pending.background).toBe('pending.png');
    expect(pending.regions[0].target).toBe('A edit');
    expect(pending.revision).toBe(2);
    const next = scopedDraft(saved, pending, { regionId: 'b' });
    expect(next.regions[0].target).toBe('A edit');
    expect(next.regions[1].source).toBe('B edit');
  });
  it('discards only A and accepts newer job data without losing edits to B', () => {
    const base = page(),
      draft = structuredClone(base),
      latest = structuredClone(base);
    draft.regions[0].target = 'discard';
    draft.regions[1].target = 'keep';
    latest.revision = 8;
    latest.regions[0].source = 'new OCR';
    latest.regions[1].style.color = '#ff0000';
    const result = remainingDraft(base, draft, latest, { regionId: 'a' });
    expect(result.regions[0]).toEqual(latest.regions[0]);
    expect(result.regions[1]).toMatchObject({ target: 'keep', style: { color: '#ff0000' } });
    expect(changedRegions(latest, result)).toEqual(['b']);
  });
  it('handles additions and deletions without resurrecting other removed regions', () => {
    const base = page(),
      draft = structuredClone(base);
    draft.regions.push(region('c'));
    draft.regions.splice(0, 1);
    const deletion = scopedDraft(base, draft, { regionId: 'a' });
    expect(deletion.regions.map((r) => r.id)).toEqual(['b']);
    const pending = remainingDraft(base, draft, { ...deletion, revision: 2 }, { regionId: 'a' });
    expect(changedRegions(deletion, pending)).toEqual(['c']);
    const discarded = remainingDraft(deletion, pending, deletion, { regionId: 'c' });
    expect(hasDraftChanges(deletion, discarded)).toBe(false);
    const stale = structuredClone(base);
    stale.regions[1].target = 'stale';
    expect(() =>
      remainingDraft(base, stale, { ...base, regions: [base.regions[0]] }, { regionId: 'a' }),
    ).toThrow('removed');
  });
  it('applies an imported background independently while keeping text drafts', () => {
    const base = page(),
      draft = structuredClone(base);
    draft.background = 'import.png';
    draft.regions[1].target = 'pending';
    const sent = scopedDraft(base, draft, { background: true });
    expect(sent.regions).toEqual(base.regions);
    const saved = { ...sent, revision: 2, background: 'managed-clean.png' };
    const result = remainingDraft(base, draft, saved, { background: true });
    expect(result.background).toBe('managed-clean.png');
    expect(changedRegions(saved, result)).toEqual(['b']);
  });
});

describe('durable save acknowledgements', () => {
  it('retains the newer checkpoint for both scoped and whole-page acknowledgements', () => {
    const base = page(), draft = structuredClone(base);
    draft.regions[0].target = 'saved A'; draft.regions[1].target = 'pending B';
    const acknowledged = { ...scopedDraft(base, draft, { regionId: 'a' }), revision: 2 };
    const latest = structuredClone(acknowledged);
    latest.revision = 3; latest.rendered = 'checkpoint.png';
    latest.regions[0].source = 'new OCR'; latest.regions[1].style.color = '#ff0000';
    const result = acknowledgeDraft(base, draft, acknowledged, latest, { regionId: 'a' });
    expect(result.saved).toEqual(latest);
    expect(result.pending?.regions[0].source).toBe('new OCR');
    expect(result.pending?.regions[1]).toMatchObject({ target: 'pending B', style: { color: '#ff0000' } });
    expect(changedRegions(result.baseline, result.pending!)).toEqual(['b']);
    expect(acknowledgeDraft(base, draft, acknowledged, latest).saved).toEqual(latest);
    expect(acknowledgeDraft(base, draft, acknowledged, latest).pending).toBeNull();
  });
  it('keeps unresolved drafts after a durable save even when a remaining region was removed', () => {
    const base = page(), draft = structuredClone(base);
    draft.regions[0].target = 'saved A'; draft.regions[1].target = 'unresolved B';
    const acknowledged = { ...scopedDraft(base, draft, { regionId: 'a' }), revision: 2 };
    const latest = { ...acknowledged, revision: 3, regions: [acknowledged.regions[0]], rendered: 'latest.png' };
    const result = acknowledgeDraft(base, draft, acknowledged, latest, { regionId: 'a' });
    expect(result.saved).toEqual(latest); expect(String(result.warning)).toContain('removed');
    expect(result.pending?.regions[1].target).toBe('unresolved B');
    expect(changedRegions(result.baseline, result.pending!)).toEqual(['b']);
    expect(result.pending?.revision).toBe(3);
    expect(() => mergePageEdits(result.baseline, result.pending!, latest)).toThrow('removed');
    expect(hasDraftChanges(result.baseline, result.pending!)).toBe(true);
  });
});
