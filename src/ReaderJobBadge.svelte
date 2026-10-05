<script lang="ts">
  import { type UiText, t, tr, count, locale } from './i18n';
  import type { JobAction } from './commands';
  import type { Job } from './types';
  import { AlertCircle } from 'lucide-svelte';
  import JobTimer from './JobTimer.svelte';
  import PageJobProgress from './PageJobProgress.svelte';
  import Modal from './Modal.svelte';
  import { jobProgress, statusLabels, jobHasWarning } from './jobs-state';
  let {
    job,
    pending,
    held,
    available,
    reason,
    hint,
    oncontrol,
    onsettings,
  }: {
    job: Job | null;
    pending: number;
    held: boolean;
    available: boolean;
    reason: UiText;
    hint: UiText;
    oncontrol: (id: string, action: JobAction) => Promise<void>;
    onsettings: () => void;
  } = $props();
  let open = $state(false);
</script>

<!-- A dedicated gutter row, outside the reading canvas, never overlays page pixels. -->
<div class="reader-progress-gutter">
  <button
    class="reader-job-badge"
    title={t('m_a8535af37d36', $locale)}
    aria-label={t('m_415bf2bb2450', $locale)}
    onclick={() => (open = true)}
  >
    <span class="reader-stage" role="status" aria-live="polite" aria-atomic="true"
      >{job
        ? job.stopping
          ? t('m_bbe8574175ab', $locale)
          : job.status === 'running'
            ? tr(jobProgress(job).label, $locale)
            : tr(statusLabels[job.status], $locale)
        : t('m_5fa7aac5375c', $locale)}</span
    >
    <span class="warning-slot"
      >{#if job && jobHasWarning(job)}<AlertCircle size={12} />{/if}</span
    >
    {#if job}<JobTimer {job} />{/if}{#if pending}<span>+{pending}</span>{/if}
  </button>
</div>
{#if open}<Modal title={t('m_a083f6b8c1e0', $locale)} onclose={() => (open = false)}>
    {#if job}<PageJobProgress
        {job}
        {held}
        {available}
        {reason}
        {hint}
        {oncontrol}
        onsettings={() => {
          open = false;
          onsettings();
        }}
        expanded={true}
      />{:else}<p>{t('m_b54f5c0f97d9', $locale)}</p>{/if}
    {#if pending}<small
        >{count(pending, 'm_142d86ecdb18', 'm_4fc3a311a68f', $locale)}
        {tr('in this chapter.', $locale)}</small
      >{/if}
  </Modal>{/if}

<style>
  .reader-stage {
    width: 13em;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-align: right;
  }
  .warning-slot {
    display: flex;
    width: 12px;
    flex: none;
    color: #a66b13;
  }
  .reader-progress-gutter {
    flex: none;
    min-height: 27px;
    display: flex;
    justify-content: flex-end;
    align-items: center;
    background: var(--window);
    padding: 1px 10px;
    border-top: 1px solid var(--border);
  }
  .reader-job-badge {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 0.7rem;
    min-height: 23px;
    padding: 2px 7px;
    border: 0;
    background: transparent;
    color: var(--muted);
  }
</style>
