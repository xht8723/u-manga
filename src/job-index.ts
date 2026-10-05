import type { Job } from './types';

type Tab = 'ongoing' | 'completed' | 'stopped';
const tab = (j: Job): Tab => j.status === 'complete' ? 'completed' : ['failed', 'cancelled'].includes(j.status) ? 'stopped' : 'ongoing';
const active = (j: Job) => ['running', 'queued', 'paused'].includes(j.status);
const key = (path: string, page: string) => `${path}\u0000${page}`;
const indexes = new WeakMap<Job[], JobIndex>();

/** Incremental presentation indexes; event ordering remains owned by JobCollection. */
export class JobIndex {
  private snapshot?: Job[];
  private pages = new Map<string, Map<string, Job>>();
  private books = new Map<string, Map<string, Job>>();
  private tabs: Record<Tab, Map<string, Job>> = { ongoing: new Map(), completed: new Map(), stopped: new Map() };
  runningOrWaiting = 0;
  stopping = 0;
  startable = 0;
  clear() { this.pages.clear(); this.books.clear(); for (const rows of Object.values(this.tabs)) rows.clear(); this.runningOrWaiting = this.stopping = this.startable = 0; }
  remove(job: Job) {
    this.tabs[tab(job)].delete(job.id);
    for (const [groups, group] of [[this.pages, key(job.project, job.pageId)], [this.books, job.location?.bookId || job.project]] as const) {
      const rows = groups.get(group); rows?.delete(job.id); if (!rows?.size) groups.delete(group);
    }
    if (['running', 'queued'].includes(job.status)) this.runningOrWaiting--;
    if (job.stopping) this.stopping--;
    if (['queued', 'paused', 'failed'].includes(job.status)) this.startable--;
  }
  add(job: Job) {
    this.tabs[tab(job)].set(job.id, job);
    for (const [groups, group] of [[this.pages, key(job.project, job.pageId)], [this.books, job.location?.bookId || job.project]] as const) {
      let rows = groups.get(group); if (!rows) groups.set(group, rows = new Map()); rows.set(job.id, job);
    }
    if (['running', 'queued'].includes(job.status)) this.runningOrWaiting++;
    if (job.stopping) this.stopping++;
    if (['queued', 'paused', 'failed'].includes(job.status)) this.startable++;
  }
  bind(jobs: Job[]) { if (this.snapshot) indexes.delete(this.snapshot); this.snapshot = jobs; indexes.set(jobs, this); return jobs; }
  latest(path?: string, page?: string) {
    if (!path || !page) return null;
    let latest: Job | null = null;
    for (const job of this.pages.get(key(path, page))?.values() || []) {
      if (!latest || Number(active(job)) > Number(active(latest)) ||
        (active(job) === active(latest) && (job.created > latest.created ||
          (job.created === latest.created && (job.sequence || 0) > (latest.sequence || 0))))) latest = job;
    }
    return latest;
  }
  candidates(selectedTab: Tab, book = '') {
    const rows = book ? this.books.get(book)?.values() || [] : this.tabs[selectedTab].values();
    return [...rows].filter(j => tab(j) === selectedTab);
  }
  counts() { return { ongoing:this.tabs.ongoing.size, completed:this.tabs.completed.size, stopped:this.tabs.stopped.size }; }
  pending(path: string | undefined, pages: ReadonlySet<string>, current?: string) {
    if (!path) return 0;
    let count = 0;
    for (const id of pages) {
      if (id === current) continue;
      for (const job of this.pages.get(key(path, id))?.values() || []) if (active(job)) count++;
    }
    return count;
  }
}
export function jobIndex(jobs: Job[]) {
  let index = indexes.get(jobs);
  if (!index) { index = new JobIndex(); jobs.forEach(j => index!.add(j)); index.bind(jobs); }
  return index;
}
