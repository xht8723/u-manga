import { describe, it, expect } from 'vitest';
import {
  handles,
  beginGesture,
  updateGesture,
  imagePoint,
  moveBox,
  setRegionBox,
  geometry,
  applyGeometryEdit,
  type GeometryEdit,
} from './region-editing';
import { mergePageEdits } from './editor-state';
import type { Page, Region } from './types';
const region = (): Region => ({
  id: 'region-a',
  bbox: [20, 30, 80, 90],
  bubble: [10, 10, 100, 100],
  kind: 'text',
  score: 1,
  source: 'source',
  target: 'translation',
  direction: 'auto',
  style: { font: null, size: null, color: '#000000', fill: '#ffffff', lineGap: 0.2, outlineEnabled: false, outlineWidthPercent: 8, outlineColor: '#ffffff' },
  allowFill: false,
  overlayOnly: false,
  prepared: false,
  review: null,
});
const page = (): Page => ({ id: 'page-a', width: 200, height: 300, regions: [region()] }) as Page;
describe('region gestures', () => {
  it('maps zoomed and scrolled view coordinates to source pixels', () => {
    expect(
      imagePoint([160, 120], { left: 60, top: -30, width: 100, height: 150 }, 200, 300),
    ).toEqual([200, 300]);
  });
  it('previews drawing in all directions without mutating page or creating zero boxes', () => {
    const p = page(),
      before = structuredClone(p);
    for (const end of [
      [10, 20],
      [130, 20],
      [10, 200],
      [130, 200],
    ]) {
      const g = beginGesture(p, null, 'draw', 1, [70, 90], [70, 90]);
      const next = updateGesture(g, end as [number, number], end as [number, number]);
      expect(next.box).toEqual([
        Math.min(70, end[0]),
        Math.min(90, end[1]),
        Math.max(70, end[0]),
        Math.max(90, end[1]),
      ]);
      expect(next.meaningful).toBe(true);
      expect(g.box).toEqual([70, 90, 70, 90]);
    }
    expect(p).toEqual(before);
    const click = beginGesture(p, null, 'draw', 1, [70, 90], [70, 90]);
    expect(updateGesture(click, [71, 91], [71, 91]).meaningful).toBe(false);
  });
  it('moves without resizing and clamps at every page edge', () => {
    expect(moveBox([20, 30, 80, 90], -500, -500, 200, 300)).toEqual([0, 0, 60, 60]);
    expect(moveBox([20, 30, 80, 90], 500, 500, 200, 300)).toEqual([140, 240, 200, 300]);
  });
  it.each(handles)(
    'resizes %s on the correct axes and cannot invert or leave the page',
    (handle) => {
      const g = beginGesture(page(), region(), handle, 1, [20, 30], [20, 30]);
      const b = updateGesture(g, [30, 40], [30, 40]).box;
      expect(b).toEqual([
        handle.includes('w') ? 30 : 20,
        handle.includes('n') ? 40 : 30,
        handle.includes('e') ? 90 : 80,
        handle.includes('s') ? 100 : 90,
      ]);
      for (const point of [
        [-999, -999],
        [999, 999],
      ] as [number, number][]) {
        const next = updateGesture(g, point, point).box;
        expect(next[0]).toBeGreaterThanOrEqual(0);
        expect(next[1]).toBeGreaterThanOrEqual(0);
        expect(next[2]).toBeLessThanOrEqual(200);
        expect(next[3]).toBeLessThanOrEqual(300);
        expect(next[2] - next[0]).toBeGreaterThanOrEqual(1);
        expect(next[3] - next[1]).toBeGreaterThanOrEqual(1);
      }
    },
  );
  it('validates numeric edits and clears only an outgrown balloon association', () => {
    const r = region();
    expect(setRegionBox(r, [30, 30, 90, 90], 200, 300)).toBe(true);
    expect(r.bubble).not.toBeNull();
    expect(setRegionBox(r, [80, 80, 160, 180], 200, 300)).toBe(true);
    expect(r.bubble).toBeNull();
    const before = structuredClone(r);
    expect(setRegionBox(r, [NaN, 0, 0, 0], 200, 300)).toBe(false);
    expect(r).toEqual(before);
  });
  it('geometry undo preserves unrelated edits and newly published OCR', () => {
    const p = page(),
      before = geometry(p.regions[0]);
    setRegionBox(p.regions[0], [120, 120, 180, 180], 200, 300);
    const entry: GeometryEdit = {
      pageId: p.id,
      regionId: 'region-a',
      before,
      after: geometry(p.regions[0]),
      index: 0,
    };
    p.regions[0].target = 'manual edit';
    p.regions[0].source = 'new OCR';
    expect(applyGeometryEdit(p, entry, true)).toBe(true);
    expect(p.regions[0].bbox).toEqual(before.bbox);
    expect(p.regions[0].target).toBe('manual edit');
    expect(p.regions[0].source).toBe('new OCR');
    expect(applyGeometryEdit(p, entry, false)).toBe(true);
    expect(p.regions[0].bubble).toBeNull();
    p.regions = [];
    expect(applyGeometryEdit(p, entry, true)).toBe(false);
    p.id = 'another';
    expect(applyGeometryEdit(p, entry, true)).toBe(false);
  });
  it('undo/redo of a drawn region retains edits made after drawing', () => {
    const p = page(),
      r = p.regions[0];
    const entry: GeometryEdit = {
      pageId: p.id,
      regionId: r.id,
      before: null,
      after: geometry(r),
      created: structuredClone(r),
      index: 0,
    };
    r.target = 'new lettering';
    expect(applyGeometryEdit(p, entry, true)).toBe(true);
    expect(p.regions).toHaveLength(0);
    expect(applyGeometryEdit(p, entry, false)).toBe(true);
    expect(p.regions[0].target).toBe('new lettering');
  });
  it('merges moved geometry with latest fields without retaining stale detector bounds', () => {
    const base = page(),
      draft = structuredClone(base),
      latest = structuredClone(base);
    draft.regions[0].bbox = [120, 120, 180, 180];
    latest.regions[0].source = 'new OCR';
    const result = mergePageEdits(base, draft, latest);
    expect(result.regions[0].bubble).toBeNull();
    expect(result.regions[0].source).toBe('new OCR');
  });
});
