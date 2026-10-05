import { describe, it, expect } from 'vitest';
import { acceptsRegionResult } from './region-result';
import type { Page, RegionRequest, RegionResult } from './types';
const request = {
  id: 'operation',
  path: 'book',
  pageId: 'page',
  expected: 3,
  region: { id: 'region', bbox: [0, 0, 10, 20], source: 'corrected', target: 'old' },
} as RegionRequest;
const result: RegionResult = {
  id: 'operation',
  pageId: 'page',
  regionId: 'region',
  expected: 3,
  text: 'new',
  elapsedMs: 40,
};
const page = () =>
  ({ id: 'page', revision: 3, regions: [structuredClone(request.region)] }) as Page;
describe('region draft ownership', () => {
  it('accepts only the captured identity, revision and current inputs', () => {
    expect(acceptsRegionResult(request, result, page(), 'translate')).toBe(true);
    for (const change of [{ id: 'other' }, { revision: 4 }, { regions: [] }])
      expect(
        acceptsRegionResult(request, result, { ...page(), ...change } as Page, 'translate'),
      ).toBe(false);
    for (const field of ['id', 'pageId', 'regionId'] as const)
      expect(acceptsRegionResult(request, { ...result, [field]: 'other' }, page(), 'read')).toBe(
        false,
      );
    const p = page();
    p.regions[0].bbox[2] = 11;
    expect(acceptsRegionResult(request, result, p, 'read')).toBe(false);
  });
  it('preserves newer source/target edits but permits unrelated changes', () => {
    const p = page();
    p.regions[0].source = 'newer';
    expect(acceptsRegionResult(request, result, p, 'read')).toBe(false);
    p.regions[0].source = request.region.source;
    p.regions[0].target = 'manual';
    expect(acceptsRegionResult(request, result, p, 'read')).toBe(true);
    expect(acceptsRegionResult(request, result, p, 'translate')).toBe(false);
    p.regions[0].target = request.region.target;
    p.regions[0].direction = 'vertical';
    expect(acceptsRegionResult(request, result, p, 'translate')).toBe(true);
  });
});
