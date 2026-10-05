import type { Page } from './types';
import { mergePageEdits } from './editor-state';

export type EditScope = { regionId: string } | { background: true };
export type EditorCommit = { before: Page; after: Page; scope?: EditScope };
const equal = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);

export function changedRegions(base: Page, draft: Page): string[] {
  const original = new Map(base.regions.map((r) => [r.id, r]));
  const pending = new Map(draft.regions.map((r) => [r.id, r]));
  return [...new Set([...original.keys(), ...pending.keys()])].filter(
    (id) => !equal(original.get(id), pending.get(id)),
  );
}
export function hasDraftChanges(base: Page, draft: Page): boolean {
  return base.background !== draft.background || changedRegions(base, draft).length > 0;
}

/** Copy one region, including additions/deletions, without touching any other region. */
function copyRegion(to: Page, from: Page, id: string) {
  const index = to.regions.findIndex((r) => r.id === id);
  const region = from.regions.find((r) => r.id === id);
  if (index >= 0) {
    if (region) to.regions[index] = structuredClone(region);
    else to.regions.splice(index, 1);
  } else if (region) {
    to.regions.splice(
      Math.min(from.regions.indexOf(region), to.regions.length),
      0,
      structuredClone(region),
    );
  }
}

/** The existing native merge/render path receives only the user's chosen scope. */
export function scopedDraft(base: Page, draft: Page, scope: EditScope): Page {
  const result = structuredClone(base);
  if ('regionId' in scope) copyRegion(result, draft, scope.regionId);
  else result.background = draft.background;
  return result;
}

function unresolvedScope(base: Page, draft: Page, saved: Page, scope: EditScope) {
  const prior = structuredClone(base), pending = structuredClone(draft);
  if ('regionId' in scope) {
    copyRegion(prior, saved, scope.regionId);
    copyRegion(pending, saved, scope.regionId);
  } else {
    prior.background = saved.background;
    pending.background = saved.background;
  }
  return { prior, pending };
}

/** Clear just the resolved scope, then merge remaining drafts over the new saved revision. */
export function remainingDraft(base: Page, draft: Page, saved: Page, scope: EditScope): Page {
  const { prior, pending } = unresolvedScope(base, draft, saved, scope);
  return mergePageEdits(prior, pending, saved);
}

/** A delayed acknowledgement must never roll back a newer owned checkpoint. */
export function acknowledgeDraft(
  base: Page, draft: Page, acknowledged: Page, current: Page, scope?: EditScope,
): { saved: Page; pending: Page | null; baseline: Page; warning?: unknown } {
  const saved = structuredClone(
    current.id === acknowledged.id && current.revision > acknowledged.revision
      ? current : acknowledged,
  );
  if (!scope) return { saved, pending: null, baseline: saved };
  const { prior, pending } = unresolvedScope(base, draft, saved, scope);
  try {
    return { saved, pending: mergePageEdits(prior, pending, saved), baseline: saved };
  } catch (warning) {
    // Clear the durable scope but retain unresolved intentions against their old
    // fields. Native merge/readiness still rejects removed regions; Discard works.
    prior.revision = saved.revision;
    pending.revision = saved.revision;
    return { saved, pending, baseline: prior, warning };
  }
}
