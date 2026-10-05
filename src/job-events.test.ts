import { describe, it, expect } from 'vitest';
import { JobCollection } from './job-events';
import type { Job, JobsBatch } from './types';
const job = (id: string, sequence: number, status: Job['status'] = 'queued') =>
  ({ id, sequence, status }) as Job;
const batch = (version: number, jobs: Job[], extra: Partial<JobsBatch> = {}): JobsBatch => ({
  version,
  jobs,
  removed: [],
  generation: 1,
  held: false,
  recovering: false,
  errors: [],
  ...extra,
});
describe('batched job events', () => {
  it('merges late history without overwriting newer progress or hold state', () => {
    const state = new JobCollection();
    state.apply(batch(2, [job('active', 4, 'paused')], { held: true }));
    const jobs = state.apply(batch(1, [job('active', 1), job('history', 2, 'complete')]));
    expect(jobs.find((j) => j.id === 'active')?.status).toBe('paused');
    expect(jobs).toHaveLength(2);
    expect(state.held).toBe(true);
  });
  it('does not recreate deleted jobs or mix libraries', () => {
    const state = new JobCollection();
    state.apply(batch(1, [job('deleted', 1)]));
    state.apply(batch(3, [], { removed: ['deleted'] }));
    expect(state.apply(batch(2, [job('deleted', 2)]))).toHaveLength(0);
    state.apply(batch(4, [job('new', 3)], { generation: 2 }));
    expect(state.apply(batch(9, [job('old', 8)]))).toMatchObject([job('new', 3)]);
  });
  it('updates stopping and renamed location metadata at the same durable sequence', () => {
    const state = new JobCollection();
    state.apply(batch(1, [{ ...job('one', 4), stopping: true }]));
    expect(state.apply(batch(2, [{ ...job('one', 4), stopping: false }]))[0].stopping).toBe(false);
  });
});
