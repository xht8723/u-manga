/** Loaded only by the explicitly requested, isolated packaged regression harness. */
import { listen } from '@tauri-apps/api/event';
import { call } from './bridge';
export async function startJobsSmoke() {
  await call('jobs_smoke_ui_report', { report: { stage: 'bootstrap received' } });
  const gaps: number[] = [];
  let previous = 0,
    sampling = true;
  const frame = (time: number) => {
    if (previous) gaps.push(time - previous);
    previous = time;
    if (sampling) requestAnimationFrame(frame);
  };
  requestAnimationFrame(frame);
  const stop = await listen('jobs-smoke-finished', () => {
    sampling = false;
    stop();
    void call('jobs_smoke_ui_report', {
      report: {
        frames: gaps.length,
        maxFrameGapMs: gaps.reduce((max, n) => Math.max(max, n), 0),
        meanFrameGapMs: gaps.reduce((a, b) => a + b, 0) / Math.max(1, gaps.length),
        renderedJobCards: document.querySelectorAll('.queue-job').length,
      },
    });
  });
  return () => {
    sampling = false;
    stop();
  };
}
