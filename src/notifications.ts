import { writable } from 'svelte/store';
import { sameUiText, type UiText } from './i18n';

export type NoticeSeverity = 'success' | 'info' | 'warning' | 'error';
export type Notice = { id: string; message: UiText; severity: NoticeSeverity; repeats: number };
export type NoticeSnapshot = {
  visible: readonly Notice[];
  queued: readonly Notice[];
  covered: boolean;
  announcement: (Notice & { serial: number }) | null;
};
export type NoticeClock = {
  now(): number;
  set(callback: () => void, milliseconds: number): unknown;
  clear(handle: unknown): void;
};
const defaultClock: NoticeClock = {
  now: () => performance.now(),
  set: (callback, milliseconds) => setTimeout(callback, milliseconds),
  clear: handle => clearTimeout(handle as ReturnType<typeof setTimeout>),
};
type Entry = {
  notice: Notice;
  remaining: number;
  started: number | null;
  timer: unknown;
  generation: number;
  pauses: Set<string>;
};
let sessionSequence = 0;
/** One mounted app owns this queue. Clocks never depend on locale or UI renders. */
export class Notifications {
  private readonly session = ++sessionSequence;
  private sequence = 0;
  private serial = 0;
  private entries = new Map<string, Entry>();
  private visible: string[] = [];
  private queued: string[] = [];
  private coverage = new Set<string>(['unmounted']);
  private announcement: NoticeSnapshot['announcement'] = null;
  private pendingAnnouncements = new Map<string, Notice>();
  private destroyed = false;
  readonly state = writable<NoticeSnapshot>({ visible: [], queued: [], covered: true, announcement: null });
  constructor(private clock: NoticeClock = defaultClock) {}

  publish(severity: NoticeSeverity, message: UiText): string | undefined {
    if (this.destroyed || !message) return;
    const duplicate = [...this.entries.values()].find(entry =>
      entry.notice.severity === severity && sameUiText(entry.notice.message, message));
    if (duplicate) {
      this.stop(duplicate);
      duplicate.notice = { ...duplicate.notice, repeats: duplicate.notice.repeats + 1 };
      duplicate.remaining = 5000;
      if (this.visible.includes(duplicate.notice.id)) {
        this.announce(duplicate.notice);
        this.start(duplicate);
      }
      this.emit();
      return duplicate.notice.id;
    }
    const id = `notice-${this.session}-${++this.sequence}`;
    const notice = { id, severity, message: structuredClone(message), repeats: 1 };
    const entry: Entry = { notice, remaining: 5000, started: null, timer: undefined, generation: 0, pauses: new Set() };
    this.entries.set(id, entry);
    if (this.visible.length < 3) {
      this.visible.push(id);
      this.announce(notice);
      this.start(entry);
    } else this.queued.push(id);
    this.emit();
    return id;
  }
  /** Capture the caller's page/session ownership before an asynchronous operation. */
  scope(current: () => boolean) {
    return (severity: NoticeSeverity, message: UiText) => current() ? this.publish(severity, message) : undefined;
  }
  dismiss(id: string) {
    const entry = this.entries.get(id);
    if (!entry) return;
    this.stop(entry);
    this.entries.delete(id);
    this.pendingAnnouncements.delete(id);
    this.visible = this.visible.filter(value => value !== id);
    this.queued = this.queued.filter(value => value !== id);
    while (this.visible.length < 3 && this.queued.length) {
      const next = this.entries.get(this.queued.shift()!)!;
      this.visible.push(next.notice.id);
      this.announce(next.notice);
      this.start(next);
    }
    this.emit();
  }
  pause(id: string, reason: string, paused: boolean) {
    const entry = this.entries.get(id);
    if (!entry || entry.pauses.has(reason) === paused) return;
    this.stop(entry);
    if (paused) entry.pauses.add(reason); else entry.pauses.delete(reason);
    this.start(entry);
  }
  cover(reason: string, covered: boolean) {
    if (this.destroyed || this.coverage.has(reason) === covered) return;
    for (const id of this.visible) this.stop(this.entries.get(id)!);
    if (covered) this.coverage.add(reason); else this.coverage.delete(reason);
    for (const id of this.visible) this.start(this.entries.get(id)!);
    this.emit();
  }
  destroy() {
    if (this.destroyed) return;
    this.destroyed = true;
    for (const entry of this.entries.values()) this.stop(entry);
    this.entries.clear(); this.visible = []; this.queued = []; this.announcement = null;
    this.pendingAnnouncements.clear();
    this.coverage.clear(); this.coverage.add('destroyed'); this.emit();
  }
  private stop(entry: Entry) {
    entry.generation++;
    if (entry.started !== null) entry.remaining = Math.max(0, entry.remaining - (this.clock.now() - entry.started));
    entry.started = null;
    if (entry.timer !== undefined) this.clock.clear(entry.timer);
    entry.timer = undefined;
  }
  private start(entry: Entry) {
    if (this.destroyed || this.coverage.size || entry.pauses.size || !this.visible.includes(entry.notice.id) ||
      entry.notice.severity === 'warning' || entry.notice.severity === 'error') return;
    const generation = ++entry.generation;
    entry.started = this.clock.now();
    entry.timer = this.clock.set(() => {
      if (this.entries.get(entry.notice.id) !== entry || entry.generation !== generation || this.destroyed) return;
      this.dismiss(entry.notice.id);
    }, entry.remaining);
  }
  takeAnnouncements(): readonly Notice[] {
    const pending = [...this.pendingAnnouncements.values()]; this.pendingAnnouncements.clear(); return pending;
  }
  private announce(notice: Notice) {
    this.pendingAnnouncements.set(notice.id, notice);
    this.announcement = { ...notice, serial: ++this.serial };
  }
  private emit() {
    this.state.set({
      visible: this.visible.map(id => this.entries.get(id)!.notice),
      queued: this.queued.map(id => this.entries.get(id)!.notice),
      covered: !!this.coverage.size,
      announcement: this.announcement,
    });
  }
}
