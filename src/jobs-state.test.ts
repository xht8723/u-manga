import { describe, it, expect } from 'vitest';
import { filterJobs, jobTab, jobProgress, canResumeJob, type JobsFilters } from './jobs-state';
import { settingsDefaults } from './book-state';
import type { Job } from './types';

const filters: JobsFilters = { query: '', book: '', chapter: '', status: '', stage: '' };
function job(id: string, status: Job['status'], overrides: Partial<Job> = {}): Job {
  return {
    id,
    kind: 'translation',
    steps: [],
    pageRevision: null,
    status,
    project: 'book.umanga',
    pageId: id,
    stage: status === 'complete' ? 'done' : 'translating',
    stageDetail: '',
    error: null,
    created: `2026-09-24T10:00:0${id}Z`,
    elapsedMs: 0,
    settings: settingsDefaults().translation,
    provider: {
      name: 'Provider',
      model: 'Vision model',
      service: 'llm',
    },
    location: {
      bookId: 'b',
      bookTitle: '図書館',
      chapterId: 'c',
      chapterTitle: 'Chapter 2',
      pageNumber: 7,
      chapterPages: 20,
    },
    ...overrides,
  };
}
describe('jobs groups, filters, and step reporting', () => {
  it('only resumes paused or failed jobs', () => {
    expect(
      (['queued', 'running', 'paused', 'complete', 'failed', 'cancelled'] as const).map((status) =>
        canResumeJob({ status }),
      ),
    ).toEqual([false, false, true, false, true, false]);
  });
  it('keeps paused jobs ongoing and separates successful and unsuccessful terminal states', () => {
    const jobs = (['queued', 'running', 'paused', 'complete', 'failed', 'cancelled'] as const).map(
      (s, i) => job(String(i), s),
    );
    expect(jobs.map(jobTab)).toEqual([
      'ongoing',
      'ongoing',
      'ongoing',
      'completed',
      'stopped',
      'stopped',
    ]);
    const grouped = ['ongoing', 'completed', 'stopped'].flatMap((tab) =>
      filterJobs(jobs, tab as any, filters),
    );
    expect(new Set(grouped.map((j) => j.id)).size).toBe(jobs.length);
  });
  it('combines chapter-relative page search with book, chapter, status and step filters', () => {
    const wanted = job('1', 'running'),
      other = job('2', 'paused');
    expect(
      filterJobs([wanted, other], 'ongoing', {
        query: 'PAGE 7',
        book: 'b',
        chapter: 'c',
        status: 'running',
        stage: 'translating',
      }),
    ).toEqual([wanted]);
    expect(filterJobs([wanted], 'ongoing', { ...filters, query: '図書館' })).toHaveLength(1);
    expect(filterJobs([wanted], 'ongoing', { ...filters, chapter: 'different' })).toHaveLength(0);
    expect(filterJobs([wanted], 'ongoing', { ...filters, book: 'different' })).toHaveLength(0);
  });
  it('prioritizes active work and orders completed history newest first without mutating the input', () => {
    const jobs = [
      job('1', 'paused'),
      job('2', 'queued'),
      job('3', 'running'),
      job('4', 'complete'),
      job('5', 'complete'),
    ];
    expect(filterJobs(jobs, 'ongoing', filters).map((j) => j.id)).toEqual(['3', '2', '1']);
    expect(filterJobs(jobs, 'completed', filters).map((j) => j.id)).toEqual(['5', '4']);
    expect(jobs[0].status).toBe('paused');
  });
  it('counts real stages separately for vision and local OCR, including retained failed steps', () => {
    const vision = job('1', 'failed');
    vision.settings.mode = 'vision';
    expect(jobProgress(vision)).toMatchObject({ current: 3, label: 'Reading & translating' });
    const local = { ...vision, settings: { ...vision.settings, mode: 'local' } };
    expect(jobProgress(local)).toMatchObject({ current: 4, label: 'Translating' });
    expect(jobProgress({ ...local, stage: 'ocr' }).current).toBe(3);
    expect(jobProgress({ ...local, status: 'complete', stage: 'done' }).current).toBe(7);
    expect(jobProgress({ ...vision, kind: 'cleanup', stage: 'lettering' })).toMatchObject({
      current: 3,
      steps: ['loading', 'cleaning', 'lettering', 'saving'],
    });
    expect(jobProgress({ ...vision, status: 'queued', stage: 'waiting' }).current).toBe(0);
  });
  it('retains jobs with missing source metadata without inventing chapter or page numbers', () => {
    const missing = job('1', 'failed', { location: null });
    expect(filterJobs([missing], 'stopped', filters)).toEqual([missing]);
    expect(filterJobs([missing], 'stopped', { ...filters, query: 'page 7' })).toEqual([]);
    expect(filterJobs([missing], 'stopped', { ...filters, book: missing.project })).toEqual([
      missing,
    ]);
  });
});

it('keeps glossary workflow stable and warns without changing completion grouping', () => {
  const task = job('8', 'running');
  task.settings.autoGlossary = true;
  task.settings.glossaryEnabled = true;
  task.settings.mode = 'vision';
  task.stage = 'glossary';
  const steps = jobProgress(task).steps;
  expect(steps).toEqual([
    'loading',
    'detecting',
    'transcribing',
    'glossary',
    'translating',
    'cleaning',
    'lettering',
    'saving',
  ]);
  expect(jobProgress(task).label).toBe('Finding terms');
  task.steps.push({ stage: 'glossary', status: 'warning', detail: 'Offline', error: null });
  task.stage = 'translating';
  expect(jobProgress(task).steps).toEqual(steps);
  expect(jobProgress(task).label).toBe('Reading & translating');
  task.status = 'complete';
  expect(jobTab(task)).toBe('completed');
  expect(jobProgress(task).steps).toEqual(steps);
  task.settings.autoGlossary = false;
  expect(jobProgress(task).steps).not.toContain('glossary');
});
