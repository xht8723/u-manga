import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { get } from 'svelte/store';
import { Notifications, type NoticeClock } from './notifications';
import { setLanguage, type UiText } from './i18n';
let notices: Notifications;
const clock: NoticeClock = { now: () => Date.now(), set: (callback, delay) => setTimeout(callback, delay), clear: handle => clearTimeout(handle as ReturnType<typeof setTimeout>) };
const snapshot = () => get(notices.state);
beforeEach(() => { vi.useFakeTimers(); vi.setSystemTime(0); notices = new Notifications(clock); notices.cover('unmounted', false); });
afterEach(() => { notices.destroy(); vi.useRealTimers(); setLanguage('en'); });
describe('session notification ownership and clocks', () => {
  it('dismisses info and success after five visible seconds', () => {
    notices.publish('info', 'Jobs added'); notices.publish('success', 'Export complete.');
    vi.advanceTimersByTime(4999); expect(snapshot().visible).toHaveLength(2);
    vi.advanceTimersByTime(1); expect(snapshot().visible).toEqual([]);
  });
  it('retains warnings and errors until manually dismissed', () => {
    const warning = notices.publish('warning', 'Saved with warnings.')!;
    notices.publish('error', 'Failure'); vi.advanceTimersByTime(600_000);
    expect(snapshot().visible).toHaveLength(2); expect(vi.getTimerCount()).toBe(0);
    notices.dismiss(warning); expect(snapshot().visible.map(n => n.severity)).toEqual(['error']);
  });
  it('runs three independent timers and starts FIFO promotions at visibility', () => {
    notices.publish('info', 'First'); vi.advanceTimersByTime(1000);
    notices.publish('info', 'Second'); vi.advanceTimersByTime(1000);
    notices.publish('success', 'Third'); notices.publish('info', 'Fourth'); notices.publish('info', 'Fifth');
    expect(vi.getTimerCount()).toBe(3); expect(snapshot().queued.map(n => n.message)).toEqual(['Fourth', 'Fifth']);
    vi.advanceTimersByTime(3000); expect(snapshot().visible.map(n => n.message)).toEqual(['Second', 'Third', 'Fourth']);
    vi.advanceTimersByTime(1000); expect(snapshot().visible.map(n => n.message)).toEqual(['Third', 'Fourth', 'Fifth']);
    vi.advanceTimersByTime(3999); expect(snapshot().visible.map(n => n.message)).toEqual(['Fourth', 'Fifth']);
    vi.advanceTimersByTime(1); expect(snapshot().visible.map(n => n.message)).toEqual(['Fifth']);
    vi.advanceTimersByTime(1000); expect(snapshot().visible).toEqual([]);
  });
  it('queues behind three persistent notices without silently evicting them', () => {
    for (let i = 0; i < 3; i++) notices.publish('error', `Failure ${i}`);
    const pending = notices.publish('info', 'Queued')!; vi.advanceTimersByTime(60000);
    expect(snapshot().visible).toHaveLength(3); expect(snapshot().queued[0].id).toBe(pending);
    notices.dismiss(snapshot().visible[1].id); expect(snapshot().visible[2].id).toBe(pending);
    vi.advanceTimersByTime(4999); expect(snapshot().visible).toHaveLength(3);
    vi.advanceTimersByTime(1); expect(snapshot().visible).toHaveLength(2);
  });
  it('coalesces deserialized full content, retaining distinct diagnostics and severity', () => {
    const text: UiText = { key: 'unknown', args: {}, fallback: 'Failure', detail: 'Diagnostic A' };
    const id = notices.publish('warning', text);
    expect(notices.publish('warning', JSON.parse(JSON.stringify(text)))).toBe(id);
    notices.publish('warning', { ...text, detail: 'Diagnostic B' });
    notices.publish('success', text); notices.publish('error', text);
    expect(snapshot().visible[0].repeats).toBe(2);
    expect(snapshot().visible.map(n => n.severity)).toEqual(['warning', 'warning', 'success']);
    expect(snapshot().queued[0].severity).toBe('error');
  });
  it('refreshes a visible repeat for five seconds without changing identity', () => {
    const id = notices.publish('info', 'Jobs added'); vi.advanceTimersByTime(4000);
    expect(notices.publish('info', 'Jobs added')).toBe(id); expect(snapshot().visible[0].repeats).toBe(2);
    vi.advanceTimersByTime(4999); expect(snapshot().visible[0].id).toBe(id);
    vi.advanceTimersByTime(1); expect(snapshot().visible).toEqual([]);
  });
  it('coalesces queued repeats and gives them a full clock when promoted', () => {
    for (let i = 0; i < 3; i++) notices.publish('error', String(i));
    const id = notices.publish('info', 'Queued'); vi.advanceTimersByTime(10000);
    expect(notices.publish('info', 'Queued')).toBe(id); expect(snapshot().queued[0].repeats).toBe(2);
    notices.dismiss(snapshot().visible[0].id); vi.advanceTimersByTime(4999);
    expect(snapshot().visible.at(-1)?.id).toBe(id); vi.advanceTimersByTime(1); expect(snapshot().visible).toHaveLength(2);
  });
  it.each(['hover', 'focus', 'touch-7'])('pauses during %s and resumes only the remaining duration', reason => {
    const id = notices.publish('success', 'Saved')!; vi.advanceTimersByTime(2000);
    notices.pause(id, reason, true); vi.advanceTimersByTime(60000);
    expect(snapshot().visible).toHaveLength(1); notices.pause(id, reason, false);
    vi.advanceTimersByTime(2999); expect(snapshot().visible).toHaveLength(1);
    vi.advanceTimersByTime(1); expect(snapshot().visible).toHaveLength(0);
  });
  it('keeps overlapping focus/touch/hover pause reasons independent', () => {
    const id = notices.publish('info', 'Saved')!; vi.advanceTimersByTime(1000);
    notices.pause(id, 'hover', true); notices.pause(id, 'focus', true); notices.pause(id, 'touch-2', true);
    notices.pause(id, 'touch-2', false); notices.pause(id, 'hover', false); vi.advanceTimersByTime(6000);
    expect(snapshot().visible).toHaveLength(1); notices.pause(id, 'focus', false);
    vi.advanceTimersByTime(4000); expect(snapshot().visible).toEqual([]);
  });
  it('pauses hidden, modal, busy and hosted login coverage without resetting elapsed time', () => {
    const id = notices.publish('info', 'Saved')!; vi.advanceTimersByTime(2000);
    for (const reason of ['document', 'modal', 'busy', 'login']) notices.cover(reason, true);
    vi.advanceTimersByTime(100000);
    for (const reason of ['document', 'modal', 'busy']) notices.cover(reason, false);
    vi.advanceTimersByTime(10000); expect(snapshot().covered).toBe(true);
    notices.cover('login', false); vi.advanceTimersByTime(2999); expect(snapshot().visible[0].id).toBe(id);
    vi.advanceTimersByTime(1); expect(snapshot().visible).toEqual([]);
  });
  it('refreshes a paused repeat but waits for coverage and focus to end', () => {
    const id = notices.publish('info', 'Saved')!; vi.advanceTimersByTime(4000);
    notices.cover('modal', true); notices.pause(id, 'focus', true); notices.publish('info', 'Saved');
    notices.cover('modal', false); vi.advanceTimersByTime(6000); expect(snapshot().visible).toHaveLength(1);
    notices.pause(id, 'focus', false); vi.advanceTimersByTime(4999); expect(snapshot().visible).toHaveLength(1);
    vi.advanceTimersByTime(1); expect(snapshot().visible).toEqual([]);
  });
  it('does not reset clocks on locale changes or unrelated snapshots', () => {
    notices.publish('info', 'Export complete.'); vi.advanceTimersByTime(3000);
    setLanguage('zh-Hans'); notices.publish('warning', 'Other');
    vi.advanceTimersByTime(2000); expect(snapshot().visible.map(n => n.severity)).toEqual(['warning']);
  });
  it('ignores stale timer callbacks after refresh, pause, dismissal and teardown', () => {
    const callbacks: (() => void)[] = []; let now = 0;
    const controller = new Notifications({ now: () => now, set: callback => { callbacks.push(callback); return callbacks.length; }, clear: () => {} });
    controller.cover('unmounted', false); const id = controller.publish('info', 'First')!;
    now = 1000; controller.publish('info', 'First'); callbacks[0](); expect(get(controller.state).visible).toHaveLength(1);
    controller.cover('hidden', true); callbacks[1](); expect(get(controller.state).visible).toHaveLength(1);
    controller.cover('hidden', false); controller.dismiss(id); controller.publish('info', 'Next');
    callbacks[2](); expect(get(controller.state).visible[0].message).toBe('Next');
    controller.destroy(); callbacks.at(-1)!(); expect(get(controller.state).visible).toEqual([]);
  });
  it('rejects disposed and superseded publishers while preserving notices across navigation', () => {
    let generation = 1, alive = true; const captured = generation;
    const owner = notices.scope(() => alive && generation === captured);
    owner('info', 'Existing'); generation++; owner('error', 'Late old page');
    notices.scope(() => alive)('warning', 'New page'); alive = false;
    notices.scope(() => alive)('error', 'Disposed');
    expect(snapshot().visible.map(n => n.message)).toEqual(['Existing', 'New page']);
  });
  it('owns no live timers before mounting and clears all state and timers on teardown', () => {
    notices.cover('unmounted', true); notices.publish('info', 'Before mount');
    vi.advanceTimersByTime(10000); expect(snapshot().visible).toHaveLength(1); expect(vi.getTimerCount()).toBe(0);
    notices.cover('unmounted', false); expect(vi.getTimerCount()).toBe(1);
    notices.destroy(); expect(vi.getTimerCount()).toBe(0); expect(snapshot().queued).toEqual([]);
    expect(notices.publish('error', 'After teardown')).toBeUndefined();
  });
  it('announces each visible message once, retaining batched arrivals and delaying covered messages', () => {
    notices.publish('info', 'First'); notices.publish('error', 'Second'); notices.publish('warning', 'Third');
    expect(notices.takeAnnouncements().map(n => n.message)).toEqual(['First', 'Second', 'Third']);
    expect(notices.takeAnnouncements()).toEqual([]); vi.advanceTimersByTime(1000); expect(notices.takeAnnouncements()).toEqual([]);
    notices.cover('login', true); notices.publish('info', 'Fourth'); expect(notices.takeAnnouncements()).toEqual([]);
    notices.dismiss(snapshot().visible[0].id); expect(snapshot().covered).toBe(true);
    expect(notices.takeAnnouncements().map(n => n.message)).toEqual(['Fourth']);
  });
});
