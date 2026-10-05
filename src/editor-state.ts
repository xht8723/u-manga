import type { Job, Page } from './types';
import { contains } from './region-editing';
import { jobIndex } from './job-index';

const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);
/** Keep local field edits while accepting unrelated pipeline checkpoints. */
export function mergePageEdits(base: Page, draft: Page, latest: Page): Page {
  if (base.id !== draft.id || draft.id !== latest.id) throw Error('Editor page changed');
  const result = structuredClone(latest);
  if (base.background !== draft.background) result.background = draft.background;
  result.regions = result.regions.filter(
    (r) => !base.regions.some((b) => b.id === r.id) || draft.regions.some((d) => d.id === r.id),
  );
  for (const [index, edited] of draft.regions.entries()) {
    const old = base.regions.find((r) => r.id === edited.id);
    if (!old) {
      if (result.regions.some((r) => r.id === edited.id)) throw Error('Region ID already exists');
      // Restore an intentionally deleted region beside its next surviving neighbor.
      // New pipeline regions retain their relative ordering.
      const next = draft.regions
        .slice(index + 1)
        .find((r) => result.regions.some((saved) => saved.id === r.id));
      const insertion = next
        ? result.regions.findIndex((r) => r.id === next.id)
        : result.regions.length;
      result.regions.splice(insertion, 0, structuredClone(edited));
      continue;
    }
    if (same(old, edited)) continue;
    const current = result.regions.find((r) => r.id === edited.id);
    if (!current) throw Error('An edited region was removed. Discard this draft to reload.');
    for (const key of [
      'bbox',
      'source',
      'target',
      'direction',
      'allowFill',
      'overlayOnly',
    ] as const) {
      if (!same(old[key], edited[key]))
        Object.assign(current, { [key]: structuredClone(edited[key]) });
    }
    for (const key of ['font', 'size', 'color', 'fill', 'lineGap', 'outlineEnabled', 'outlineWidthPercent', 'outlineColor'] as const) {
      if (!same(old.style[key], edited.style[key]))
        Object.assign(current.style, { [key]: edited.style[key] });
    }
    if (!same(old.bbox, edited.bbox) && current.bubble && !contains(current.bubble, current.bbox))
      current.bubble = null;
  }
  return result;
}
export function latestPageJob(jobs: Job[], path: string | undefined, pageId: string | undefined) {
  return jobIndex(jobs).latest(path, pageId);
}
