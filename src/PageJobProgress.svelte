<script lang="ts">
  import { uiError, sameUiText, type UiText, t, tr, locale } from './i18n';
  import type { JobAction } from './commands';
  import {
    Check,
    LoaderCircle,
    AlertCircle,
    Pause,
    Play,
    RotateCcw,
    X,
    ChevronDown,
    ChevronUp,
  } from 'lucide-svelte';
  import JobTimer from './JobTimer.svelte';
  import RequirementHint from './RequirementHint.svelte';
  import type { Job } from './types';
  import { jobProgress, statusLabels, jobStageLabel } from './jobs-state';
  let {
    job,
    held = false,
    oncontrol,
    available = false,
    reason = '',
    hint = '',
    onsettings = () => {},
    expanded = $bindable(false),
    showHeading = true,
    compact = false,
  }: {
    job: Job;
    held?: boolean;
    oncontrol: (id: string, action: JobAction) => Promise<void>;
    available?: boolean;
    reason?: UiText;
    hint?: UiText;
    onsettings?: () => void;
    expanded?: boolean;
    showHeading?: boolean;
    compact?: boolean;
  } = $props();
  let pending = $state(false),
    error = $state<UiText>('');
  let progress = $derived(jobProgress(job));
  async function control(action: JobAction) {
    pending = true;
    error = '';
    try {
      await oncontrol(job.id, action);
    } catch (e) {
      error = uiError(e);
    } finally {
      pending = false;
    }
  }
</script>

<section class="page-job" aria-label={t('m_537e44798637', $locale)}>
  {#if showHeading}<div class="page-job-heading">
      <button
        class="page-job-summary"
        aria-expanded={expanded}
        title={tr(expanded ? 'Collapse job details' : 'Show job steps and errors', $locale)}
        onclick={() => (expanded = !expanded)}
      >
        <span
          ><strong
            >{job.kind === 'cleanup'
              ? t('m_9f1b237b5dc3', $locale)
              : job.kind === 'preparation'
                ? t('m_cf2befb0f1a6', $locale)
                : t('m_94debb41f8e7', $locale)}</strong
          ><small
            >{job.stopping
              ? t('m_bbe8574175ab', $locale)
              : job.status === 'running'
                ? tr(progress.label, $locale)
                : tr(statusLabels[job.status], $locale)}</small
          ></span
        >
        <JobTimer {job} />{#if expanded}<ChevronUp size={15} />{:else}<ChevronDown size={15} />{/if}
      </button>
    </div>{/if}
  {#if !expanded && job.error}<p class="page-job-error-summary" title={tr(job.error, $locale)}>
      {tr(job.error, $locale)}
    </p>{/if}
  {#if expanded}
    <div class="page-job-actions">
      {#if ['queued', 'running'].includes(job.status)}
        <button
          aria-label={t('m_93a0bb2c2ebb', $locale)}
          title={t('m_93a0bb2c2ebb', $locale)}
          disabled={pending || job.stopping}
          onclick={() => control('pause')}><Pause size={14} /></button
        >
      {:else if ['paused', 'failed'].includes(job.status)}
        <button
          aria-label={tr(
            job.status === 'failed' ? 'Retry this page’s job' : 'Resume this page’s job',
            $locale,
          )}
          title={tr(
            held ? 'Use Start all in Jobs first' : reason || 'Continue from saved results',
            $locale,
          )}
          disabled={pending || job.stopping || held || !available}
          onclick={() => control(job.status === 'failed' ? 'retry' : 'resume')}
          >{#if job.status === 'failed'}<RotateCcw size={14} />{:else}<Play
              size={14}
            />{/if}</button
        >
      {/if}
      {#if ['running', 'queued', 'paused'].includes(job.status)}<button
          aria-label={t('m_031906a7cc6a', $locale)}
          title={t('m_b42c4d23fb6d', $locale)}
          disabled={pending}
          onclick={() => control('cancel')}><X size={14} /></button
        >{/if}
    </div>
    {#if ['paused', 'failed'].includes(job.status)}<RequirementHint
        reason={held ? 'Use Start all in Jobs first.' : hint}
        {onsettings}
      />{/if}
    <ol class="page-job-steps">
      {#each progress.steps as stage (stage)}
        {@const step = job.steps.find((s) => s.stage === stage)}
        <li
          class:step-error={step?.status === 'failed' || step?.status === 'warning'}
          aria-current={job.stage === stage ? 'step' : undefined}
        >
          <span class="step-icon"
            >{#if step?.status === 'complete' || step?.status === 'skipped'}<Check
                size={13}
              />{:else if step?.status === 'failed' || step?.status === 'warning'}<AlertCircle
                size={13}
              />{:else if step?.status === 'running' && job.status === 'running'}<LoaderCircle
                size={13}
                class="spin"
              />{:else}·{/if}</span
          >
          <div>
            <span>{tr(jobStageLabel(job, stage), $locale)}</span>
            <small class="step-detail"
              >{tr(step?.detail || (compact ? '' : '\u00a0'), $locale)}</small
            >
            {#if step?.error}<p class="error-text">{tr(step.error, $locale)}</p>{/if}
          </div>
        </li>
      {/each}
    </ol>
    {#if job.error && !progress.steps.some((stage) => sameUiText(job.steps.find((s) => s.stage === stage)?.error, job.error))}<p class="error-text">
        {tr(job.error, $locale)}
      </p>{/if}
    {#if error}<p class="error-text" role="alert">{tr(error, $locale)}</p>{/if}
  {/if}
</section>

<style>
  .page-job-summary {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 0;
    border: 0;
    background: transparent;
    text-align: left;
    font-size: 1em;
  }
  .page-job-summary > span:first-child {
    min-width: 0;
    flex: 1;
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .page-job-summary small {
    color: var(--muted);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .page-job-error-summary {
    margin: 7px 0 0;
    font-size: 0.8em;
    color: var(--danger);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    overflow-wrap: anywhere;
  }
</style>
