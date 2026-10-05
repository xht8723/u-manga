import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Job, TranslationBatchResult } from './types';
vi.mock('./bridge', () => ({ native: false, call: vi.fn() }));
import { call } from './bridge';
import { submitJobs, submitPreparation, submitTranslationBatch } from './job-events';
const rpc = vi.mocked(call);
function deferred<T>() {
  let resolve!: (value: T) => void, reject!: (error: Error) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
const result: TranslationBatchResult = {
  jobs: [],
  warnings: [],
  skipped: 0,
  omitted: 0,
  busy: 0,
  changed: 0,
  held: false,
};
afterEach(() => vi.resetAllMocks());
describe('in-flight submissions', () => {
  it('still coalesces identical ordinary Reader submissions', async () => {
    const pending = deferred<{ jobs: Job[]; warnings: []; held: boolean }>();
    rpc.mockReturnValue(pending.promise as never);
    const first = submitJobs('ordinary', ['page']);
    const second = submitJobs('ordinary', ['page']);
    expect(rpc).toHaveBeenCalledTimes(1);
    pending.resolve({ jobs: [], warnings: [], held: false });
    await Promise.all([first, second]);
  });
  it('never merges Skip/Replace/preparation with a pending ordinary request', async () => {
    const pending = deferred<{ jobs: Job[]; warnings: []; held: boolean }>();
    rpc.mockReturnValue(pending.promise as never);
    const first = submitJobs('conflict', ['page']);
    for (const mode of ['skip_translated', 'replace'] as const)
      await expect(
        submitTranslationBatch('conflict', 'book', null, [{ id: 'page', revision: 0 }], mode),
      ).rejects.toThrow('already being submitted');
    await expect(submitPreparation('conflict', 'page', 0, true)).rejects.toThrow(
      'already being submitted',
    );
    expect(rpc).toHaveBeenCalledTimes(1);
    pending.resolve({ jobs: [], warnings: [], held: false });
    await first;
  });
  it('exclusive batches reject Reader and conflicting batch requests and release on failure', async () => {
    const pending = deferred<TranslationBatchResult>();
    rpc.mockReturnValue(pending.promise as never);
    const first = submitTranslationBatch(
      'batch',
      'book',
      null,
      [{ id: 'page', revision: 0 }],
      'replace',
    );
    const failed = expect(first).rejects.toThrow('disk failed');
    await expect(submitJobs('batch', ['page'])).rejects.toThrow('already being submitted');
    await expect(
      submitTranslationBatch(
        'batch',
        'book',
        null,
        [{ id: 'page', revision: 0 }],
        'skip_translated',
      ),
    ).rejects.toThrow('already being submitted');
    pending.reject(new Error('disk failed'));
    await failed;
    rpc.mockResolvedValue(result as never);
    await expect(
      submitTranslationBatch(
        'batch',
        'book',
        null,
        [{ id: 'page', revision: 0 }],
        'skip_translated',
      ),
    ).resolves.toEqual(result);
  });
});
