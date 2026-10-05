<script lang="ts">
  import { t, tr, locale } from './i18n';
  import { jobClock, activeElapsed, duration } from './job-clock';
  import type { Job } from './types';
  let { job }: { job: Job } = $props();
</script>

<span
  class="job-timer"
  title={t('m_2584360b884d', $locale)}
  aria-label={t('m_662524e7c9bd', $locale)}>{duration(activeElapsed(job, $jobClock))}</span
>

<style>
  .job-timer {
    flex: none;
    min-width: 4.5ch;
    text-align: right;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
</style>
