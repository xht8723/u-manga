import { readable } from 'svelte/store';
import type { Job } from './types';

// One subscription clock shared by visible timers. No IPC or database queries.
export const jobClock = readable(performance.now(), (set) => {
  set(performance.now());
  const interval = setInterval(() => set(performance.now()), 1000);
  return () => clearInterval(interval);
});
export function activeElapsed(job: Job, now: number) {
  return job.elapsedMs + (job.timerRunning ? Math.max(0, now - (job.receivedAt ?? now)) : 0);
}
export function duration(ms: number) {
  const seconds = Math.floor(Math.max(0, ms) / 1000);
  const minutes = Math.floor(seconds / 60);
  return minutes >= 60
    ? `${Math.floor(minutes / 60)}:${String(minutes % 60).padStart(2, '0')}:${String(seconds % 60).padStart(2, '0')}`
    : `${minutes}:${String(seconds % 60).padStart(2, '0')}`;
}
