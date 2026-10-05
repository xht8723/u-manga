import type { UiText } from './i18n';
import { listen } from '@tauri-apps/api/event';
import { call, native } from './bridge';
import { JobIndex } from './job-index';
export { jobIndex } from './job-index';
import type {
  Job,
  JobsBatch,
  PageVersion,
  TranslationBatchMode,
  TranslationBatchResult,
  JobSubmission,
} from './types';
/** Committed jobs remain usable when a subsequent durable state write reports a warning. */
export function submittedJobs(receipt: JobSubmission): Job[] {
  const warnings = new Map(receipt.warnings.map(w => [w.id, w.message]));
  return receipt.jobs.map(job => warnings.has(job.id) ? { ...job, error: warnings.get(job.id)! } : job);
}

/** Per-job versions allow a late initial snapshot to fill history without reverting newer events. */
export class JobCollection {
  private index = new JobIndex();
  private entries = new Map<string, { job: Job; version: number }>();
  private removed = new Map<string, number>();
  generation = -1;
  version = -1;
  held = false;
  recovering = false;
  errors: UiText[] = [];
  apply(batch: JobsBatch): Job[] {
    if (batch.generation < this.generation) return this.values();
    if (batch.generation > this.generation) {
      this.entries.clear();
      this.index.clear();
      this.removed.clear();
      this.version = -1;
      this.generation = batch.generation;
    }
    for (const id of batch.removed) {
      const entry = this.entries.get(id);
      if (entry && entry.version <= batch.version) { this.index.remove(entry.job); this.entries.delete(id); }
      this.removed.set(id, Math.max(this.removed.get(id) ?? -1, batch.version));
    }
    for (const job of batch.jobs) {
      if ((this.removed.get(job.id) ?? -1) >= batch.version) continue;
      const previous = this.entries.get(job.id);
      const sequence = job.sequence ?? 0,
        older = previous?.job.sequence ?? 0;
      if (
        !previous ||
        sequence > older ||
        (sequence === older && batch.version >= previous.version)
      ) {
        if (previous) this.index.remove(previous.job);
        const received = { ...job, receivedAt: performance.now() };
        this.index.add(received);
        this.entries.set(job.id, {
          job: received,
          version: batch.version,
        });
      }
    }
    if (batch.version >= this.version) {
      this.version = batch.version;
      this.held = batch.held;
      this.recovering = batch.recovering;
      this.errors = batch.errors;
    }
    return this.values();
  }
  values() {
    return this.index.bind([...this.entries.values()].map((entry) => entry.job));
  }
}
export async function listenJobs(callback: (batch: JobsBatch) => void): Promise<() => void> {
  if (native) return listen<JobsBatch>('jobs', ({ payload }) => callback(payload));
  const listener = (event: Event) => callback((event as CustomEvent<JobsBatch>).detail);
  window.addEventListener('umanga-jobs', listener);
  return () => window.removeEventListener('umanga-jobs', listener);
}
type PendingSubmission = { kind: 'ordinary'; request: Promise<Job[]> } | { kind: 'exclusive' };
const submissions = new Map<string, PendingSubmission>();
export async function submitJobs(
  path: string,
  pageIds: string[],
  priority = false,
  commands: typeof call = call,
): Promise<Job[]> {
  const ids = [...new Set(pageIds)];
  const key = (id: string) => `${path}\u0000${id}`;
  if (ids.some((id) => submissions.get(key(id))?.kind === 'exclusive'))
    throw new Error('This page is already being submitted.');
  const waiting = new Set(
    ids.flatMap((id) => {
      const entry = submissions.get(key(id));
      return entry?.kind === 'ordinary' ? [entry.request] : [];
    }),
  );
  const fresh = ids.filter((id) => !submissions.has(key(id)));
  if (fresh.length) {
    const request = commands('enqueue', { path, pageIds: fresh, priority }).then(submittedJobs).finally(() => {
      for (const id of fresh) submissions.delete(key(id));
    });
    for (const id of fresh) submissions.set(key(id), { kind: 'ordinary', request });
    waiting.add(request);
  }
  return [...new Map((await Promise.all(waiting)).flat().map((job) => [job.id, job])).values()];
}

export async function submitTranslationBatch(
  path: string,
  bookId: string,
  chapterId: string | null,
  pages: PageVersion[],
  mode: TranslationBatchMode,
): Promise<TranslationBatchResult> {
  const keys = pages.map((p) => `${path}\u0000${p.id}`);
  if (keys.some((key) => submissions.has(key)))
    throw new Error('This page is already being submitted.');
  for (const key of keys) submissions.set(key, { kind: 'exclusive' });
  try {
    return await call('translation_batch_submit', { path, bookId, chapterId, pages, mode });
  } finally {
    for (const key of keys) submissions.delete(key);
  }
}

export type CleanupSubmission = JobSubmission & { untranslated: number; busy: number };
export async function submitPreparation(
  path: string,
  pageId: string,
  expected: number,
  replace: boolean,
  commands: typeof call = call,
): Promise<Job[]> {
  const key = `${path}\u0000${pageId}`;
  if (submissions.has(key)) throw new Error('This page is already being submitted.');
  const request = commands('prepare_enqueue', { path, pageId, expected, replace }).then(submittedJobs).finally(() =>
    submissions.delete(key),
  );
  submissions.set(key, { kind: 'exclusive' });
  return request;
}
