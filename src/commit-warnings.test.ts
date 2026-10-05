import { afterEach, describe, expect, it, vi } from 'vitest';
import { tr, uiJoin, type UiText } from './i18n';
import { warningGroup, warningMessage } from './commit-warnings';
import { Notifications } from './notifications';
import { get } from 'svelte/store';
import { batchFeedback } from './batch-translation';
const native = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({
  invoke: native.invoke,
  isTauri: () => true,
  convertFileSrc: (path: string) => path,
}));
import { call, scopedCommands, beginWarningSession } from './bridge';
afterEach(() => {
  vi.unstubAllGlobals();
  native.invoke.mockReset();
});
describe('commit warning presentation', () => {
  it('dispatches plain, structured and joined messages without consuming receipt IDs', async () => {
    const target = new EventTarget();
    vi.stubGlobal('window', target);
    const details: UiText[] = [];
    target.addEventListener('umanga-commit-warning', (event) =>
      details.push((event as CustomEvent).detail),
    );
    const structured: UiText = { key: 'test-fallback', args: {}, fallback: 'Save all' };
    const messages = ['preserved diagnostic', structured, uiJoin(['first', structured])];
    const receipt = {
      jobs: [],
      held: false,
      warnings: messages.map((message, index) => ({ id: 'job-' + index, message })),
    };
    native.invoke.mockResolvedValue(receipt);
    const result = await call('enqueue', { path: 'fixture.umanga', pageIds: ['page'] });
    expect(result).toBe(receipt);
    expect(result.warnings.map((w) => w.id)).toEqual(['job-0', 'job-1', 'job-2']);
    expect(details).toEqual([warningGroup(receipt.warnings)]);
    for (const language of ['en', 'zh-Hans'] as const)
      expect(tr(details[0], language)).toBe(tr(warningGroup(receipt.warnings), language));
    expect(tr(details[0])).not.toContain('[object Object]');
  });
  it('retains direct book warning messages through the same boundary', () => {
    const direct = { key: 'test-fallback', args: {}, fallback: 'Discard all' };
    expect(warningMessage(direct)).toBe(direct);
  });
  it('groups identical deserialized warnings but keeps different diagnostics and all receipt data', async () => {
    const target = new EventTarget();
    vi.stubGlobal('window', target);
    const details: UiText[] = [];
    target.addEventListener('umanga-commit-warning', (event) =>
      details.push((event as CustomEvent).detail),
    );
    const message = {
      key: 'unknown',
      args: {},
      fallback: 'Saved partially',
      detail: 'Diagnostic A',
    };
    const receipt = Object.freeze({
      jobs: [],
      held: false,
      warnings: Object.freeze([
        Object.freeze({ id: 'a', message }),
        Object.freeze({ id: 'b', message: structuredClone(message) }),
        Object.freeze({ id: 'c', message: { ...message, detail: 'Diagnostic B' } }),
      ]),
    });
    native.invoke.mockResolvedValue(receipt);
    expect(await call('enqueue', { path: 'p', pageIds: [] })).toBe(receipt);
    expect(details).toHaveLength(1);
    expect(tr(details[0]).match(/Diagnostic A/g)).toHaveLength(1);
    expect(tr(details[0])).toContain('Diagnostic B');
    expect(receipt.warnings.map((w) => w.id)).toEqual(['a', 'b', 'c']);
  });
  it('presents a warning+success receipt once per severity without duplicating warnings in batch feedback', async () => {
    const target = new EventTarget();
    vi.stubGlobal('window', target);
    const controller = new Notifications();
    target.addEventListener('umanga-commit-warning', (event) =>
      controller.publish('warning', (event as CustomEvent).detail),
    );
    const receipt = {
      jobs: [],
      held: true,
      skipped: 1,
      omitted: 0,
      changed: 0,
      busy: 2,
      warnings: [{ id: 'a', message: 'Unique diagnostic' }],
    };
    native.invoke.mockResolvedValue(receipt);
    const result = await call('translation_batch_submit', {
      path: 'p',
      bookId: 'b',
      chapterId: null,
      pages: [],
      mode: 'replace',
    });
    controller.publish('info', batchFeedback(result));
    expect(get(controller.state).visible.map((n) => [n.severity, n.repeats])).toEqual([
      ['warning', 1],
      ['info', 1],
    ]);
    expect(tr(batchFeedback(result))).not.toContain('Unique diagnostic');
    expect(result).toBe(receipt);
    controller.destroy();
  });
  it('ignores late presentation from a stale page or replaced session without changing the command result', async () => {
    const target = new EventTarget();
    vi.stubGlobal('window', target);
    const events: unknown[] = [];
    target.addEventListener('umanga-commit-warning', (event) =>
      events.push((event as CustomEvent).detail),
    );
    let resolve!: (value: unknown) => void,
      current = true;
    native.invoke.mockImplementation(
      () =>
        new Promise((r) => {
          resolve = r;
        }),
    );
    const owned = scopedCommands(() => () => current);
    const first = owned('enqueue', { path: 'p', pageIds: [] });
    current = false;
    const receipt = { jobs: [], held: false, warnings: [{ id: 'a', message: 'Late' }] };
    resolve(receipt);
    expect(await first).toBe(receipt);
    expect(events).toEqual([]);
    const stop = beginWarningSession();
    const second = call('enqueue', { path: 'p', pageIds: [] });
    stop();
    const stopNew = beginWarningSession();
    resolve(receipt);
    expect(await second).toBe(receipt);
    expect(events).toEqual([]);
    stopNew();
  });
});
