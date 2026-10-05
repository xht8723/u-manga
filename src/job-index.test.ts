import { expect, test } from 'vitest';
import { JobCollection, submittedJobs } from './job-events';
import { jobIndex } from './job-index';
import type { Job, JobsBatch } from './types';
const row = (id: string, status: Job['status'], sequence = 1) => ({ id, project:'book',pageId:'page',created:id,status,sequence } as Job);
const batch = (version: number, jobs: Job[], removed: string[] = []): JobsBatch => ({ jobs,removed,generation:1,version,held:false,recovering:false,errors:[] });
test('incremental indexes track accepted transitions, removal, generation and old snapshots', () => {
  const state = new JobCollection(); const first = state.apply(batch(1,[row('a','queued'),row('b','complete')]));
  expect(jobIndex(first).latest('book','page')?.id).toBe('a'); expect(jobIndex(first).runningOrWaiting).toBe(1);
  const next = state.apply(batch(2,[row('a','complete',2)]));
  expect(jobIndex(next).latest('book','page')?.id).toBe('b'); expect(jobIndex(next).candidates('completed')).toHaveLength(2);
  expect(jobIndex(first).runningOrWaiting).toBe(1);
  state.apply(batch(1,[row('a','running',1)]));
  const removed = state.apply(batch(3,[],['b'])); expect(jobIndex(removed).latest('book','page')?.id).toBe('a');
  const reset = state.apply({...batch(4,[]),generation:2}); expect(jobIndex(reset).candidates('completed')).toHaveLength(0);
});
test('submission warning stays attached to its committed Job', () => {
  const jobs = submittedJobs({ jobs:[row('a','paused')],held:true,warnings:[{id:'a',message:'Disk write failed'}] });
  expect(jobs[0].error).toBe('Disk write failed');
});
