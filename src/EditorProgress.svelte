<script lang="ts">
  import { tick, onMount } from 'svelte';
  import { ChevronDown, LoaderCircle, Check, AlertCircle, Circle, Pause } from 'lucide-svelte';
  import { t, tr, locale, type UiText } from './i18n';
  import type { Job } from './types';
  import type { JobAction } from './commands';
  import type { RegionOperation } from './region-operation.svelte';
  import { jobClock, duration, activeElapsed } from './job-clock';
  import { jobProgress, statusLabels, jobHasWarning } from './jobs-state';
  import PageJobProgress from './PageJobProgress.svelte';
  import { observeViewport, placeOverlay } from './viewport';
  import { useCompactLayout } from './mobile-layout.svelte';
  const compact = useCompactLayout();
  let {
    pageId,
    job,
    operation,
    oncancel,
    held,
    available,
    reason,
    hint,
    oncontrol,
    onsettings,
  }: {
    pageId: string;
    job: Job | null;
    operation: RegionOperation | null;
    oncancel: () => void;
    held: boolean;
    available: boolean;
    reason: UiText;
    hint: UiText;
    oncontrol: (id: string, action: JobAction) => Promise<void>;
    onsettings: () => void;
  } = $props();
  let open = $state(false),
    trigger = $state<HTMLButtonElement>(),
    panel = $state<HTMLDivElement>();
  let left = $state(0),
    top = $state(0),
    width = $state(360),
    height = $state(400),
    above = $state(false);
  let active = $derived(
    operation?.pageId === pageId &&
      (!job ||
        Date.parse(job.created) <= operation.startedAt ||
        ['running', 'stopping'].includes(operation.status))
      ? operation
      : null,
  );
  let label = $derived(
    active
      ? active.action === 'read'
        ? 'Re-read text (OCR)'
        : 'Re-translate'
      : job
        ? job.kind === 'preparation'
          ? 'Manual preparation'
          : job.kind === 'cleanup'
            ? 'Re-clean page'
            : 'Auto translation'
        : 'Progress',
  );
  const stages: Record<string, string> = {
    checking: 'Checking requirements',
    loading: 'Loading page',
    ocr: 'Reading text',
    transcribing: 'Transcribing image',
    translating: 'Translating',
  };
  const states = {
    running: 'Running',
    stopping: 'Stopping…',
    complete: 'Completed',
    failed: 'Failed',
    cancelled: 'Cancelled',
  };
  let status = $derived(
    active
      ? active.status === 'running'
        ? stages[active.stage] || active.stage
        : states[active.status]
      : job
        ? job.stopping
          ? 'Stopping…'
          : job.status === 'running'
            ? jobProgress(job).label
            : statusLabels[job.status]
        : 'No page job',
  );
  let elapsed = $derived(
    active
      ? active.elapsedMs +
          (['running', 'stopping'].includes(active.status)
            ? Math.max(0, $jobClock - active.receivedAt)
            : 0)
      : job
        ? activeElapsed(job, $jobClock)
        : 0,
  );
  let running = $derived(
    active
      ? ['running', 'stopping'].includes(active.status)
      : !!job && (job.status === 'running' || job.stopping),
  );
  let failed = $derived(active ? active.status === 'failed' : job?.status === 'failed');
  let complete = $derived(active ? active.status === 'complete' : job?.status === 'complete');
  let warning = $derived(!active && !!job && jobHasWarning(job));
  let paused = $derived(!active && job?.status === 'paused');
  let identity = '';
  $effect(() => {
    if (identity !== pageId) {
      identity = pageId;
      open = false;
    }
  });
  function position() {
    if (!trigger || !panel) return;
    const r = trigger.getBoundingClientRect();
    const placed = placeOverlay(
      DOMRect.fromRect({ x: r.right - 360, y: r.top, width: 360, height: r.height }),
      360, Math.min(540, panel.scrollHeight + 2),
    );
    ({ left, top, width, above } = placed);
    height = placed.maxHeight;
  }
  function close(focus = false) {
    open = false;
    if (focus) trigger?.focus({ preventScroll: true });
  }
  let positionFrame = 0;
  function queuePosition() {
    if (!positionFrame) positionFrame = requestAnimationFrame(() => {
      positionFrame = 0;
      position();
    });
  }
  function watchPanel(node: HTMLElement) {
    const observer = new ResizeObserver(queuePosition);
    observer.observe(node);
    return { destroy: () => observer.disconnect() };
  }
  async function toggle() {
    open = !open;
    if (open) {
      await tick();
      position();
      panel?.focus({ preventScroll: true });
    }
  }
  onMount(() => {
    const outside = (e: PointerEvent) => {
      if (open && !panel?.contains(e.target as Node) && !trigger?.contains(e.target as Node))
        close();
    };
    const key = (e: KeyboardEvent) => {
      if (open && e.key === 'Escape') {
        e.preventDefault();
        e.stopImmediatePropagation();
        close(true);
      }
    };
    const observer = new ResizeObserver(queuePosition);
    if (trigger) observer.observe(trigger);
    window.addEventListener('pointerdown', outside);
    window.addEventListener('keydown', key, true);
    const stopViewport = observeViewport(position);
    window.addEventListener('scroll', position, true);
    return () => {
      cancelAnimationFrame(positionFrame);
      observer.disconnect();
      window.removeEventListener('pointerdown', outside);
      window.removeEventListener('keydown', key, true);
      stopViewport();
      window.removeEventListener('scroll', position, true);
    };
  });
</script>

<button
  class="editor-progress-trigger"
  class:compact={compact.current}
  class:running
  class:failed
  class:warning
  class:expanded={open}
  bind:this={trigger}
  aria-haspopup="dialog"
  aria-expanded={open}
  aria-label={`${tr(label, $locale)} · ${tr(status, $locale)} · ${duration(elapsed)}`}
  title={`${tr(label, $locale)} · ${tr('Show job steps and errors', $locale)}`}
  onclick={toggle}
>
  <span class="progress-icon" aria-hidden="true"
    >{#if running}<LoaderCircle size={14} class="spin" />{:else if failed || warning}<AlertCircle
        size={14}
      />{:else if complete}<Check size={14} />{:else if paused}<Pause size={14} />{:else}<Circle
        size={12}
      />{/if}</span
  >
  <span class="summary" role="status" aria-live="polite" aria-atomic="true"
    >{tr(active || job ? status : 'Progress', $locale)}</span
  >
  <span class="timer">{active || job ? duration(elapsed) : '—'}</span><ChevronDown size={13} />
</button>
{#if open}<div
    bind:this={panel}
    use:watchPanel
    role="dialog"
    aria-modal="false"
    aria-label={tr('Page progress', $locale)}
    tabindex="-1"
    class="editor-progress-popover"
    class:above
    style:left={`${left}px`}
    style:top={`${top}px`}
    style:width={`${width}px`}
    style:max-height={`${height}px`}
  >
    <div class="progress-detail-heading">
      <strong>{tr(label, $locale)}</strong><span class="timer">{duration(elapsed)}</span>
    </div>
    {#if active}
      <p role="status" class="operation-status">{tr(status, $locale)}</p>
      {#if active.error}<p class="error-text">{tr(active.error, $locale)}</p>{/if}
      {#if ['running', 'stopping'].includes(active.status)}<button
          disabled={active.status === 'stopping'}
          onclick={oncancel}>{t('m_19766ed6ccb2', $locale)}</button
        >{/if}
    {:else if job}<PageJobProgress
        {job}
        {held}
        {available}
        {reason}
        {hint}
        {oncontrol}
        {onsettings}
        expanded={true}
        showHeading={false}
        compact
      />
    {:else}<p>{tr('No page job', $locale)}</p>{/if}
  </div>{/if}

<style>
  .editor-progress-trigger.warning .progress-icon {
    color: #a66b13;
  }
  .editor-progress-trigger {
    width: 17em;
    max-width: 100%;
    display: flex;
    gap: 7px;
    align-items: center;
    flex: none;
    margin-left: auto;
    padding: 6px 10px;
    min-height: 32px;
    font-size: 0.9em;
    border-radius: 999px;
    background: var(--raised);
    color: var(--muted);
    border-color: transparent;
  }
  .editor-progress-trigger:hover,
  .editor-progress-trigger.expanded {
    border-color: var(--border);
    color: var(--text);
  }
  .editor-progress-trigger.compact {
    width: 11em;
    max-width: min(52vw, 17em);
    margin-left: 0;
    padding: 6px 8px;
    gap: 5px;
  }
  .progress-icon {
    flex: none;
    display: flex;
    align-items: center;
    color: var(--accent);
  }
  .failed .progress-icon {
    color: var(--danger);
  }
  .running .summary {
    color: var(--text);
  }
  .editor-progress-trigger :global(svg) {
    flex: none;
  }
  .editor-progress-trigger.expanded > :global(svg:last-child) {
    transform: rotate(180deg);
  }
  .summary {
    min-width: 0;
    flex: 1;
    text-align: left;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .timer {
    font-variant-numeric: tabular-nums;
    min-width: 5ch;
    text-align: right;
  }
  .editor-progress-popover {
    position: fixed;
    z-index: 90;
    width: min(360px, calc(100vw - 16px));
    overflow: auto;
    scrollbar-gutter: stable;
    padding: 16px;
    border: 1px solid var(--border);
    border-radius: 12px;
    background: var(--panel);
    color: var(--text);
    box-shadow: 0 8px 32px #0002;
    font-size: 1em;
    transform-origin: top right;
    animation: appear 130ms ease-out;
  }
  .above {
    transform-origin: bottom right;
  }
  .editor-progress-popover:focus {
    outline: none;
  }
  .progress-detail-heading {
    display: flex;
    align-items: center;
    gap: 12px;
    padding-bottom: 12px;
    border-bottom: 1px solid var(--border);
  }
  .progress-detail-heading strong {
    flex: 1;
  }
  .progress-detail-heading .timer,
  .operation-status {
    color: var(--muted);
  }
  .editor-progress-popover :global(.page-job) {
    border: 0;
    margin: 8px 0 0;
    padding: 0;
    font-size: 1em;
  }
  .editor-progress-popover :global(.page-job-steps) {
    margin-top: 4px;
  }
  .editor-progress-popover :global(.page-job-steps li) {
    padding: 7px 0;
    gap: 10px;
  }
  .editor-progress-popover :global(.step-icon) {
    padding-top: 2px;
  }
  .editor-progress-popover :global(.step-detail) {
    min-height: 0;
    margin-top: 3px;
  }
  .editor-progress-popover :global(.page-job-actions:empty) {
    display: none;
  }
  .editor-progress-popover :global(.page-job-actions) {
    justify-content: flex-end;
  }
  .editor-progress-popover :global(.page-job-actions button) {
    border: 0;
    background: var(--raised);
    border-radius: 6px;
    padding: 6px;
  }
  .editor-progress-popover :global(.step-detail:empty) {
    display: none;
  }
  @keyframes appear {
    from {
      opacity: 0;
      transform: translateY(-4px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .editor-progress-popover {
      animation: none;
    }
  }
</style>
