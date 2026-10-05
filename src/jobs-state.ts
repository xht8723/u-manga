import type { Job } from './types';
import { cleanupName } from './model-descriptions';
import { jobIndex } from './job-index';
export type JobsTab = 'ongoing' | 'completed' | 'stopped';
export type JobsFilters = {
  query: string;
  book: string;
  chapter: string;
  status: string;
  stage: string;
};
export const statusLabels: Record<Job['status'], string> = {
  queued: 'Waiting',
  running: 'Running',
  paused: 'Paused',
  complete: 'Completed',
  failed: 'Failed',
  cancelled: 'Cancelled',
};
export const stageLabels: Record<string, string> = {
  waiting: 'Waiting',
  starting: 'Starting',
  loading: 'Loading page',
  detecting: 'Detecting dialogue',
  ocr: 'Reading text locally',
  transcribing: 'Reading text with service',
  glossary: 'Detecting glossary terms',
  translating: 'Translating',
  rendering: 'Cleaning & lettering',
  cleaning: 'Cleaning',
  lettering: 'Lettering',
  saving: 'Saving result',
  done: 'Done',
};
export function canResumeJob(job: Pick<Job, 'status'>): boolean {
  return job.status === 'paused' || job.status === 'failed';
}
export function jobTab(job: Job): JobsTab {
  return job.status === 'complete'
    ? 'completed'
    : ['failed', 'cancelled'].includes(job.status)
      ? 'stopped'
      : 'ongoing';
}
export const jobBookKey = (job: Job) => job.location?.bookId || job.project;
export function detectsGlossary(job: Job) {
  return (
    job.kind === 'translation' &&
    job.provider.service === 'llm' &&
    job.settings.autoGlossary &&
    job.settings.glossaryEnabled
  );
}
export const jobHasWarning = (job: Job) => job.steps.some((s) => s.status === 'warning');
export function jobStageLabel(job: Job, stage: string, compact = false) {
  if (stage === 'glossary' && compact) return 'Finding terms';
  if (stage === 'translating' && job.settings.mode === 'vision') return 'Reading & translating';
  return stageLabels[stage] || stage;
}
export function jobProgress(job: Job) {
  const steps = [
    'loading',
    ...(job.kind === 'cleanup'
      ? []
      : [
          'detecting',
          ...(job.settings.mode === 'local' ? ['ocr'] : []),
          ...(detectsGlossary(job) && job.settings.mode === 'vision' ? ['transcribing'] : []),
          ...(detectsGlossary(job) ? ['glossary'] : []),
          ...(job.kind === 'preparation' ? [] : ['translating']),
        ]),
    'cleaning',
    ...(job.kind === 'preparation' ? [] : ['lettering']),
    'saving',
  ];
  const current =
    job.status === 'complete' ? steps.length : Math.max(0, steps.indexOf(job.stage) + 1);
  const label = jobStageLabel(job, job.stage, true);
  return { steps, current, label };
}
export function filterJobs(jobs: Job[], tab: JobsTab, filters: JobsFilters): Job[] {
  const query = filters.query.trim().toLocaleLowerCase();
  const priority: Record<Job['status'], number> = {
    running: 0,
    queued: 1,
    paused: 2,
    failed: 0,
    cancelled: 1,
    complete: 0,
  };
  return jobIndex(jobs)
    .candidates(tab, filters.book)
    .filter((job) => {
      const l = job.location;
      return (
        jobTab(job) === tab &&
        (!query ||
          `${l?.bookTitle || ''} ${l?.chapterTitle || ''} page ${l?.pageNumber ?? ''} ${job.provider.name} ${job.provider.model} ${job.kind} ${cleanupName(job.cleanupModel || job.settings.cleanup.method)}`
            .toLocaleLowerCase()
            .includes(query)) &&
        (!filters.book || jobBookKey(job) === filters.book) &&
        (!filters.chapter || l?.chapterId === filters.chapter) &&
        (!filters.status || job.status === filters.status) &&
        (!filters.stage || job.stage === filters.stage)
      );
    })
    .sort(
      (a, b) =>
        priority[a.status] - priority[b.status] ||
        (tab === 'ongoing'
          ? a.created.localeCompare(b.created)
          : b.created.localeCompare(a.created)) ||
        a.id.localeCompare(b.id),
    );
}
export function elapsedLabel(ms: number) {
  const seconds = Math.round(ms / 1000);
  return seconds < 60
    ? `${seconds}s`
    : seconds < 3600
      ? `${Math.floor(seconds / 60)}m ${seconds % 60}s`
      : `${Math.floor(seconds / 3600)}h ${Math.floor(seconds / 60) % 60}m`;
}
