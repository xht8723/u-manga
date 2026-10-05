import type { BookGlossary, JobsBatch } from './types';

/** Extraction commits are already published in Jobs; timer/stage updates need no book reads. */
export class GlossaryJobChanges {
  private generation = -1;
  private added = new Map<string, number>();
  apply(batch: JobsBatch): string[] {
    if (batch.generation < this.generation) return [];
    if (batch.generation > this.generation) {
      this.generation = batch.generation;
      this.added.clear();
    }
    const paths = new Set<string>();
    for (const job of batch.jobs) {
      const count = job.glossaryCheckpoint?.added ?? 0;
      if (count <= (this.added.get(job.id) ?? 0)) continue;
      this.added.set(job.id, count);
      paths.add(job.project);
    }
    return [...paths];
  }
}

/** Rebase safe automatic appends only. Overlapping/manual changes keep the original save guard. */
export function syncGlossary(
  baseline: BookGlossary,
  draft: BookGlossary,
  saved: BookGlossary,
): {
  baseline: BookGlossary;
  draft: BookGlossary;
  appended: number | null;
  conflict: boolean;
} | null {
  if (saved.revision <= baseline.revision) return null;
  if (JSON.stringify(draft) === JSON.stringify(baseline))
    return {
      baseline: structuredClone(saved),
      draft: structuredClone(saved),
      appended: null,
      conflict: false,
    };

  const added = saved.entries.slice(baseline.entries.length);
  const existingSources = new Set(baseline.entries.map((e) => e.source));
  const automatic = (g: BookGlossary) =>
    g.automaticSources.filter((s) => existingSources.has(s)).sort();
  const appendOnly =
    saved.enabled === baseline.enabled &&
    saved.autoDetect === baseline.autoDetect &&
    saved.deeplGlossaryId === baseline.deeplGlossaryId &&
    saved.entries.length >= baseline.entries.length &&
    baseline.entries.every(
      (e, i) => e.source === saved.entries[i].source && e.target === saved.entries[i].target,
    ) &&
    JSON.stringify(automatic(saved)) === JSON.stringify(automatic(baseline)) &&
    added.every((e) => saved.automaticSources.includes(e.source));
  const localSources = new Set(draft.entries.map((e) => e.source.trim()));
  if (!appendOnly || added.some((e) => localSources.has(e.source)))
    return { baseline, draft, appended: 0, conflict: true };

  const next = structuredClone(draft);
  next.entries.push(...structuredClone(added));
  next.automaticSources = [...new Set([...next.automaticSources, ...added.map((e) => e.source)])];
  next.revision = saved.revision;
  return { baseline: structuredClone(saved), draft: next, appended: added.length, conflict: false };
}
