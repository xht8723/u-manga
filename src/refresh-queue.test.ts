import { expect, it } from 'vitest';
import { refreshQueue } from './refresh-queue';

it('coalesces a burst into one follow-up read without blocking other pages', async () => {
  const calls: string[] = [];
  let release!: () => void;
  const queue = refreshQueue(async (key: string) => {
    calls.push(key);
    if (calls.length === 1)
      await new Promise<void>((resolve) => {
        release = resolve;
      });
  });
  const first = queue('A');
  for (let n = 0; n < 50; n++) void queue('A');
  await queue('B');
  expect(calls).toEqual(['A', 'B']);
  release();
  await first;
  expect(calls).toEqual(['A', 'B', 'A']);
  await queue('A');
  expect(calls).toHaveLength(4);
});

it('releases failed reads for a later explicit invalidation', async () => {
  let fail = true;
  const queue = refreshQueue(async () => {
    if (fail) throw Error('Disconnected');
  });
  await expect(queue('A')).rejects.toThrow('Disconnected');
  fail = false;
  await expect(queue('A')).resolves.toBeUndefined();
});
