import { expect, test } from 'vitest';
import { AsyncOwner, ownedResult } from './async-owner';
test('older responses cannot regain ownership after a newer intent or leaving', () => {
  const owner = new AsyncOwner();
  const a = owner.begin();
  const b = owner.begin();
  expect(a()).toBe(false);
  expect(b()).toBe(true);
  owner.invalidate();
  expect(b()).toBe(false);
});
test('capturing ownership does not invalidate an active intent', () => {
  const owner = new AsyncOwner(); const intent = owner.begin(), capture = owner.capture();
  expect(intent()).toBe(true); expect(capture()).toBe(true); owner.begin(); expect(capture()).toBe(false);
});
test('superseded errors are ignored while current errors remain actionable', async () => {
  const owner = new AsyncOwner();
  const first = owner.begin();
  const current = owner.begin();
  await expect(ownedResult(first, Promise.reject(Error('old')))).resolves.toBeUndefined();
  await expect(ownedResult(current, Promise.reject(Error('current')))).rejects.toThrow('current');
});
