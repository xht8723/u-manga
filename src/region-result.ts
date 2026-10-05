import type { Page, RegionRequest, RegionResult } from './types';
/** Result ownership includes the inputs and the destination field, not unrelated draft edits. */
export function acceptsRegionResult(
  request: RegionRequest,
  result: RegionResult,
  page: Page,
  action: 'read' | 'translate',
) {
  const current = page.regions.find((r) => r.id === request.region.id);
  return (
    !!current &&
    result.id === request.id &&
    result.pageId === page.id &&
    page.id === request.pageId &&
    result.regionId === current.id &&
    result.expected === request.expected &&
    page.revision === request.expected &&
    JSON.stringify(current.bbox) === JSON.stringify(request.region.bbox) &&
    current.source === request.region.source &&
    (action === 'read' || current.target === request.region.target)
  );
}
