<script lang="ts">
  import { uiError, uiJoin, type UiText, t, tr, count, locale } from './i18n';
  import type { JobAction } from './commands';
  import { cleanupName } from './model-descriptions';
  import { useCompactLayout } from './mobile-layout.svelte';
  import './mobile-jobs.css';
  import {
    AlertCircle,
    Search,
    Pause,
    Play,
    RotateCcw,
    X,
    ChevronLeft,
    ChevronRight,
    ChevronDown,
    SlidersHorizontal,
  } from 'lucide-svelte';
  import ComboBox from './ComboBox.svelte';
  import type { Job, JobsOutcome, Settings } from './types';
  import JobTimer from './JobTimer.svelte';
  import { jobIndex } from './job-index';
  import RequirementHint from './RequirementHint.svelte';
  import { jobsReadiness } from './action-readiness.svelte';
  import {
    filterJobs,
    canResumeJob,
    jobBookKey,
    jobProgress,
    statusLabels,
    stageLabels,
    jobStageLabel,
    jobHasWarning,
    type JobsTab,
  } from './jobs-state';
  let {
    historyMore = false,
    loadingHistory = false,
    onhistory,
    jobs,
    settings,
    onsettings,
    oncontrol,
    oncontrolall,
    held = false,
    recovering = false,
    errors = [],
  }: {
    historyMore?: boolean;
    loadingHistory?: boolean;
    onhistory?: () => void;
    jobs: Job[];
    settings: Settings;
    onsettings: () => void;
    held?: boolean;
    recovering?: boolean;
    errors?: UiText[];
    oncontrol: (id: string, action: JobAction) => Promise<void>;
    oncontrolall: (action: 'start' | 'stop') => Promise<JobsOutcome>;
  } = $props();
  const tabs: { id: JobsTab; label: string; title: string }[] = [
    { id: 'ongoing', label: 'Ongoing', title: 'Running, waiting, and paused jobs' },
    { id: 'completed', label: 'Completed', title: 'Successfully finished jobs' },
    { id: 'stopped', label: 'Failed / cancelled', title: 'Jobs that failed or were cancelled' },
  ];
  let tab = $state<JobsTab>('ongoing');
  let query = $state(''),
    book = $state(''),
    chapter = $state(''),
    status = $state(''),
    stage = $state('');
  let page = $state(0),
    pending = $state<string[]>([]),
    error = $state<UiText>('');
  let bulk = $state(''),
    notice = $state('');
  const compact = useCompactLayout();
  let filtersOpen = $state(false);
  let expandedJobs = $state<string[]>([]);
  const activeFilters = $derived([book, chapter, status, stage].filter(Boolean).length);
  function toggleDetails(id: string) {
    expandedJobs = expandedJobs.includes(id)
      ? expandedJobs.filter((value) => value !== id)
      : [...expandedJobs, id];
  }
  let bulkRequest = 0;
  const pageSize = 50;
  const stopping = $derived(jobIndex(jobs).stopping);
  const canStart = $derived(held || jobIndex(jobs).startable > 0);
  async function controlAll(action: 'start' | 'stop') {
    const request = ++bulkRequest;
    bulk = action;
    error = '';
    notice = '';
    try {
      const result = await oncontrolall(action);
      if (request !== bulkRequest) return;
      notice = `${result.changed} jobs ${action === 'stop' ? 'paused' : 'scheduled'}`;
      error = uiJoin(result.errors.map((e) => e.message));
    } catch (e) {
      if (request === bulkRequest) error = uiError(e);
    } finally {
      if (request === bulkRequest) bulk = '';
    }
  }
  const counts = $derived(jobIndex(jobs).counts());
  const filters = $derived({ query, book, chapter, status, stage });
  const filtered = $derived(filterJobs(jobs, tab, filters));
  const pages = $derived(Math.max(1, Math.ceil(filtered.length / pageSize)));
  const visiblePage = $derived(Math.min(page, pages - 1));
  const visible = $derived(filtered.slice(visiblePage * pageSize, (visiblePage + 1) * pageSize));
  const readiness = jobsReadiness(
    () => visible,
    () => settings,
  );
  const bookOptions = $derived(
    [
      ...new Map(
        jobs.map((job) => [
          jobBookKey(job),
          {
            value: jobBookKey(job),
            literal: !!job.location?.bookTitle,
            label: job.location?.bookTitle || 'Book unavailable',
          },
        ]),
      ).values(),
    ].sort((a, b) => a.label.localeCompare(b.label)),
  );
  const chapterOptions = $derived(
    [
      ...new Map(
        jobs
          .filter((job) => job.location && (!book || jobBookKey(job) === book))
          .map((job) => [
            job.location!.chapterId,
            {
              value: job.location!.chapterId,
              literal: true,
              label: book
                ? job.location!.chapterTitle
                : `${job.location!.bookTitle} · ${job.location!.chapterTitle}`,
            },
          ]),
      ).values(),
    ].sort((a, b) => a.label.localeCompare(b.label, undefined, { numeric: true })),
  );
  const statuses = $derived(
    tab === 'ongoing' ? ['running', 'queued', 'paused'] : ['failed', 'cancelled'],
  );
  const filterKey = $derived(JSON.stringify([tab, filters]));
  $effect(() => {
    filterKey;
    page = 0;
  });
  function selectTab(next: JobsTab) {
    tab = next;
    status = '';
    stage = '';
  }
  function tabKey(event: KeyboardEvent) {
    if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
    event.preventDefault();
    const focused = tabs.findIndex(
      (t) => `queue-tab-${t.id}` === (event.currentTarget as HTMLElement).id,
    );
    const index =
      event.key === 'Home'
        ? 0
        : event.key === 'End'
          ? tabs.length - 1
          : (focused + (event.key === 'ArrowRight' ? 1 : -1) + tabs.length) % tabs.length;
    selectTab(tabs[index].id);
    (event.currentTarget as HTMLElement).parentElement
      ?.querySelector<HTMLButtonElement>(`#queue-tab-${tab}`)
      ?.focus();
  }
  async function control(job: Job, action: JobAction) {
    pending = [...pending, job.id];
    error = '';
    try {
      await oncontrol(job.id, action);
    } catch (e) {
      error = uiError(e);
    } finally {
      pending = pending.filter((id) => id !== job.id);
    }
  }
  function clearFilters() {
    query = '';
    book = '';
    chapter = '';
    status = '';
    stage = '';
  }
</script>

<main class="queue-screen queue-view" class:compact-jobs={compact.current}>
  <div class="jobs-heading">
    <h1>{t('m_2f17a0f8d518', $locale)}</h1>
    <div class="jobs-bulk">
      <button
        title={t('m_433401753e1e', $locale)}
        disabled={bulk === 'start' || (!canStart && !recovering)}
        onclick={() => controlAll('start')}
        ><Play size={17} />{bulk === 'start'
          ? t('m_bbe5fc3b9ef3', $locale)
          : t('m_1a7e6299576a', $locale)}</button
      >
      <button
        title={t('m_98832515d7c2', $locale)}
        disabled={held || bulk === 'stop'}
        onclick={() => controlAll('stop')}><Pause size={17} />{t('m_ead4f70a6fdb', $locale)}</button
      >
    </div>
  </div>
  {#if held || stopping || recovering}<p class="jobs-state" role="status">
      {held ? t('m_d83bcc00aad6', $locale) : ''}{stopping
        ? `${held ? ' · ' : ''}Stopping ${stopping}…`
        : ''}{recovering
        ? (held || stopping ? ' · ' : '') + tr('Loading saved jobs…', $locale)
        : ''}
    </p>{/if}
  {#if notice}<p class="jobs-state" role="status">{tr(notice, $locale)}</p>{/if}
  {#each errors as issue}<p class="error-text" role="alert">{tr(issue, $locale)}</p>{/each}
  <div class="queue-tabs" role="tablist" aria-label={t('m_f77670f400a6', $locale)}>
    {#each tabs as t}<button
        role="tab"
        id={`queue-tab-${t.id}`}
        aria-controls="queue-jobs"
        aria-selected={tab === t.id}
        tabindex={tab === t.id ? 0 : -1}
        title={tr(t.title, $locale)}
        onclick={() => selectTab(t.id)}
        onkeydown={tabKey}
      >
        {tr(t.label, $locale)}<span>{counts[t.id]}</span>
      </button>{/each}
  </div>
  <div class="queue-filters" class:filters-open={filtersOpen}>
    <label class="queue-search"
      ><span>{t('m_989ecb5d07fd', $locale)}</span>
      <div>
        <Search size={16} /><input
          aria-label={t('m_989ecb5d07fd', $locale)}
          placeholder={t('m_226ef57db73c', $locale)}
          bind:value={query}
        />
      </div></label
    >
    {#if compact.current}
      <button
        class="jobs-filter-toggle"
        aria-expanded={filtersOpen}
        aria-controls="jobs-advanced-filters"
        onclick={() => (filtersOpen = !filtersOpen)}
      >
        <SlidersHorizontal size={17} /><span>{t('mobile.filters', $locale)}</span>
        {#if activeFilters}<span class="jobs-filter-count">{activeFilters}</span>{/if}
      </button>
    {/if}
    <div
      id="jobs-advanced-filters"
      class="jobs-advanced-filters"
      hidden={compact.current && !filtersOpen}
    >
      <label
        >{t('m_909cb81127c5', $locale)}<ComboBox
          label={t('m_8d216f60df04', $locale)}
          bind:value={book}
          options={[{ value: '', label: 'All books' }, ...bookOptions]}
          oncommit={() => (chapter = '')}
        /></label
      >
      <label
        >{t('m_548c31f731ec', $locale)}<ComboBox
          label={t('m_2da7539dbe4f', $locale)}
          bind:value={chapter}
          options={[{ value: '', label: 'All chapters' }, ...chapterOptions]}
        /></label
      >
      {#if tab !== 'completed'}
        <label
          >{t('m_920e413c7d41', $locale)}<ComboBox
            label={t('m_ea3391fd066b', $locale)}
            bind:value={status}
            options={[
              { value: '', label: 'All statuses' },
              ...statuses.map((s) => ({ value: s, label: statusLabels[s as Job['status']] })),
            ]}
          /></label
        >
        <label
          >{t('m_8e6a6cca7aae', $locale)}<ComboBox
            label={t('m_feb753ac4c56', $locale)}
            bind:value={stage}
            options={[
              { value: '', label: 'All steps' },
              ...Object.entries(stageLabels)
                .filter(([s]) => s !== 'done')
                .map(([value, label]) => ({ value, label })),
            ]}
          /></label
        >
      {/if}
      {#if compact.current && activeFilters}
        <button
          class="jobs-filter-clear"
          onclick={() => {
            book = '';
            chapter = '';
            status = '';
            stage = '';
          }}>{t('mobile.clear', $locale)}</button
        >
      {/if}
    </div>
  </div>
  <div class="queue-results">
    <span>{count(filtered.length, 'm_1e01744f2d62', 'm_39a17a750216', $locale)}</span
    >{#if query || book || chapter || status || stage}<button onclick={clearFilters}
        >{t('m_7179ea0035fc', $locale)}</button
      >{/if}
    {#if filtered.length > pageSize}<nav
        class="queue-pagination"
        aria-label={t('m_366320b91116', $locale)}
      >
        <button
          title={t('m_bb47c59cbbfa', $locale)}
          aria-label={t('m_bb47c59cbbfa', $locale)}
          disabled={visiblePage === 0}
          onclick={() => (page = visiblePage - 1)}><ChevronLeft size={17} /></button
        >
        <span
          >{visiblePage * pageSize + 1}–{Math.min(filtered.length, (visiblePage + 1) * pageSize)}
          {t('m_28391d3bc64e', $locale)}
          {filtered.length}</span
        >
        <button
          title={t('m_7aaef45833ea', $locale)}
          aria-label={t('m_7aaef45833ea', $locale)}
          disabled={visiblePage === pages - 1}
          onclick={() => (page = visiblePage + 1)}><ChevronRight size={17} /></button
        >
      </nav>{/if}
  </div>
  {#if error}<p class="error-text" role="alert">{tr(error, $locale)}</p>{/if}
  <div id="queue-jobs" role="tabpanel" aria-labelledby={`queue-tab-${tab}`} tabindex="0">
    {#each visible as job (job.id)}
      {@const location = job.location}
      {@const progress = jobProgress(job)}
      {@const expanded = !compact.current || expandedJobs.includes(job.id)}
      <article
        class="queue-job"
        class:job-expanded={expanded}
        aria-label={tr(
          location
            ? `${location.bookTitle}, ${location.chapterTitle}, page ${location.pageNumber}`
            : 'Job with unavailable book',
          $locale,
        )}
      >
        <div class="job-heading">
          <div class="job-identity">
            <h2 title={location?.bookTitle || job.project}>
              {location?.bookTitle || t('m_b09baa831bbc', $locale)}
            </h2>
            {#if location}<p>
                <span title={location.chapterTitle}>{location.chapterTitle}</span><span
                  class="job-page"
                  >{t('m_0a30a815d67d', $locale)}
                  {location.pageNumber} / {location.chapterPages}</span
                >
              </p>
            {:else}<p title={job.project}>{t('m_15894967c13d', $locale)}</p>{/if}
          </div>
          <span class="job-status" data-status={job.status}
            >{#if jobHasWarning(job)}<AlertCircle
                size={14}
                aria-label={tr('Completed with warnings', $locale)}
              />{/if}{job.stopping
              ? t('m_bbe8574175ab', $locale)
              : tr(statusLabels[job.status], $locale)}</span
          >
          {#if compact.current}
            <button
              class="job-details-toggle"
              aria-expanded={expanded}
              aria-controls={`job-details-${job.id}`}
              title={t(expanded ? 'mobile.hideJobDetails' : 'mobile.showJobDetails', $locale)}
              aria-label={t(expanded ? 'mobile.hideJobDetails' : 'mobile.showJobDetails', $locale)}
              onclick={() => toggleDetails(job.id)}><ChevronDown size={18} /></button
            >
          {/if}
          <div class="job-actions">
            {#if ['running', 'queued'].includes(job.status)}
              <button
                title={t('m_495faee583e8', $locale)}
                aria-label={t('m_d453d60fe822', $locale)}
                disabled={pending.includes(job.id)}
                onclick={() => control(job, 'pause')}
                ><Pause size={15} /><span>{t('m_858e4ba7a29f', $locale)}</span></button
              >
              <button
                title={t('m_a0b14e67673f', $locale)}
                aria-label={t('m_a5032d26d61d', $locale)}
                disabled={pending.includes(job.id)}
                onclick={() => control(job, 'cancel')}><X size={15} /></button
              >
            {:else if canResumeJob(job)}
              <button
                title={tr(
                  held
                    ? 'Use Start all to resume jobs'
                    : job.status === 'paused'
                      ? 'Resume this job using cached progress'
                      : 'Retry this job using cached results',
                  $locale,
                )}
                disabled={held ||
                  job.stopping ||
                  pending.includes(job.id) ||
                  !readiness.ready(job.id)}
                onclick={() => control(job, 'resume')}
              >
                {#if job.status === 'paused'}<Play size={15} /><span
                    >{t('m_d640c7421da0', $locale)}</span
                  >{:else}<RotateCcw size={15} /><span
                    >{job.status === 'failed'
                      ? t('m_942087cc2d41', $locale)
                      : t('m_6b983a81e5e8', $locale)}</span
                  >{/if}
              </button>
              {#if job.status === 'paused'}<button
                  title={t('m_e68034a5f927', $locale)}
                  aria-label={t('m_a5032d26d61d', $locale)}
                  disabled={pending.includes(job.id)}
                  onclick={() => control(job, 'cancel')}><X size={15} /></button
                >{/if}
            {/if}
          </div>
        </div>
        <div class="job-progress">
          <span class="job-stage"
            >{#if progress.current}<span class="step-count"
                >{t('m_8e6a6cca7aae', $locale)} {progress.current} / {progress.steps.length}</span
              >{/if}{tr(progress.label, $locale)}</span
          >
          {#if compact.current}<JobTimer {job} />{/if}
          {#if job.stageDetail && job.status !== 'complete' && expanded}<small
              >{tr(job.stageDetail, $locale)}</small
            >{/if}
          <div
            class="job-steps"
            role="progressbar"
            aria-label={t('m_123e6efa4bb4', $locale)}
            aria-valuemin="0"
            aria-valuemax={progress.steps.length}
            aria-valuenow={progress.current}
            aria-valuetext={tr(
              progress.current
                ? `Step ${progress.current} of ${progress.steps.length}: ${progress.label}`
                : progress.label,
              $locale,
            )}
          >
            {#each progress.steps as step, i}<span
                class:passed={job.status === 'complete' || i < progress.current - 1}
                class:current={job.status !== 'complete' && i === progress.current - 1}
                title={tr(jobStageLabel(job, step), $locale)}
                class:warning={job.steps.some((s) => s.stage === step && s.status === 'warning')}
              ></span>{/each}
          </div>
        </div>
        {#if job.error}<p class="job-error" title={tr(job.error, $locale)}>
            {tr(job.error, $locale)}
          </p>{/if}
        {#if canResumeJob(job)}<RequirementHint
            reason={held ? 'Use Start all to resume jobs.' : readiness.hint(job.id)}
            {onsettings}
          />{/if}
        <div id={`job-details-${job.id}`} class="job-expanded-details" hidden={!expanded}>
          {#if jobHasWarning(job)}<details class="job-warnings">
              <summary>{tr('Glossary warning', $locale)}</summary
              >{#each job.steps.filter((s) => s.status === 'warning') as step}<p>
                  {tr(step.detail, $locale)}
                </p>{/each}
            </details>{/if}
          <div class="job-details">
            {#if job.kind === 'translation'}<span title={job.provider.model || job.provider.service}
                >{job.provider.name}{#if job.provider.model}{' · '}{job.provider.model}{/if}</span
              >
              <span>{job.settings.sourceLanguage} → {job.settings.targetLanguage}</span>{/if}
            <span
              >{job.kind === 'cleanup'
                ? tr('Re-clean ·', $locale) + ' '
                : job.kind === 'preparation'
                  ? tr('Preparation ·', $locale) + ' '
                  : ''}{tr(
                cleanupName(job.cleanupModel || job.settings.cleanup.method),
                $locale,
              )}</span
            >
            <span
              >{job.kind === 'cleanup'
                ? t('m_9f1b237b5dc3', $locale)
                : job.settings.mode === 'local'
                  ? t('m_cde512840629', $locale)
                  : t('m_c587c2601ccf', $locale)}{#if job.device}{' · '}{tr(
                  job.device,
                  $locale,
                )}{/if}</span
            >
            {#if !compact.current}<JobTimer {job} />{/if}
          </div>
        </div>
      </article>
    {:else}<div class="empty queue-empty">
        {counts[tab]
          ? t('m_2313130d1260', $locale)
          : tab === 'ongoing'
            ? t('m_5cd641516329', $locale)
            : tab === 'completed'
              ? t('m_df9c5970f164', $locale)
              : t('m_eee731d9a1fb', $locale)}
      </div>{/each}
  </div>
  {#if historyMore}<div class="host-jobs-history">
      <button disabled={loadingHistory} onclick={onhistory}>{t('host.moreJobs', $locale)}</button>
    </div>{/if}
</main>

<style>
  .jobs-heading,
  .jobs-bulk {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .jobs-heading {
    justify-content: space-between;
  }
  .jobs-heading h1 {
    margin: 0;
  }
  .jobs-state {
    color: var(--muted);
    margin: 12px 0;
  }

  .queue-view {
    overflow-y: auto;
  }
  .queue-view h1 {
    margin-bottom: 0;
  }
  .queue-tabs {
    display: flex;
    gap: 8px;
    border-bottom: 1px solid var(--border);
    margin: 24px 0 12px;
    padding-bottom: 12px;
    flex-wrap: wrap;
  }
  .queue-tabs button {
    border-color: transparent;
    background: transparent;
  }
  .queue-tabs [aria-selected='true'] {
    background: var(--selection);
    border-color: var(--accent);
  }
  .queue-tabs span {
    font-size: 0.85em;
    color: var(--muted);
    font-variant-numeric: tabular-nums;
  }
  .queue-filters {
    display: grid;
    grid-template-columns: minmax(200px, 1.5fr) repeat(4, minmax(120px, 1fr));
    gap: 12px;
  }
  .queue-filters label {
    min-width: 0;
    margin: 4px 0;
  }
  .jobs-advanced-filters {
    display: contents;
  }
  .jobs-advanced-filters[hidden],
  .job-expanded-details[hidden] {
    display: none;
  }
  .queue-search > div {
    display: flex;
    align-items: center;
    border: 1px solid var(--border);
    border-radius: 7px;
    background: var(--panel);
    padding-left: 12px;
    min-height: 40px;
  }
  .queue-search input {
    border: 0;
    width: 100%;
    background: transparent;
  }
  .queue-search :global(svg) {
    flex: none;
    color: var(--muted);
  }
  .queue-results {
    display: flex;
    gap: 12px;
    align-items: center;
    min-height: 48px;
    color: var(--muted);
  }
  .queue-results button {
    font-size: 0.9em;
    padding: 5px 9px;
  }
  .queue-job {
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 18px 20px;
    margin-bottom: 12px;
  }
  .job-heading {
    display: flex;
    gap: 14px;
    align-items: flex-start;
  }
  .job-identity {
    min-width: 0;
    flex: 1;
  }
  .job-identity h2 {
    font-size: 1.1em;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .job-identity p {
    display: flex;
    gap: 14px;
    color: var(--muted);
    margin-top: 5px;
  }
  .job-identity p span:first-child {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .job-page {
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }
  .job-status {
    border: 1px solid var(--border);
    border-radius: 20px;
    padding: 4px 10px;
    font-size: 0.85em;
  }
  .job-status[data-status='running'] {
    color: var(--accent);
    border-color: var(--accent);
  }
  .job-status[data-status='complete'] {
    color: var(--success);
  }
  .job-status[data-status='failed'] {
    color: var(--danger);
    border-color: var(--danger);
  }
  .job-status[data-status='paused'] {
    color: var(--warning);
  }
  .job-status[data-status='cancelled'] {
    color: var(--muted);
  }
  .job-actions {
    display: flex;
    gap: 6px;
  }
  .job-actions:empty {
    display: none;
  }
  .job-actions button {
    padding: 5px 9px;
    font-size: 0.9em;
    min-height: 30px;
  }
  .job-progress {
    display: grid;
    gap: 6px;
    margin: 14px 0 10px;
  }
  .job-stage {
    display: flex;
    gap: 12px;
  }
  .step-count {
    color: var(--muted);
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }
  .job-steps {
    display: flex;
    gap: 4px;
    max-width: 450px;
  }
  .job-steps > span {
    height: 4px;
    flex: 1;
    border-radius: 3px;
    background: var(--border);
  }
  .job-steps .passed {
    background: var(--accent);
  }
  .job-steps .current {
    background: var(--accent);
    opacity: 0.55;
  }
  .job-steps .warning {
    background: #a66b13;
  }
  .job-warnings {
    margin: 8px 0;
    color: var(--muted);
    font-size: 0.85em;
  }
  .job-warnings summary {
    cursor: pointer;
  }
  .job-warnings p {
    overflow-wrap: anywhere;
  }
  .job-details {
    display: flex;
    flex-wrap: wrap;
    gap: 8px 20px;
    color: var(--muted);
    font-size: 0.85em;
  }
  .job-details > span {
    overflow-wrap: anywhere;
  }
  .job-error {
    margin: 10px 0;
    color: var(--danger);
    overflow-wrap: anywhere;
  }
  .queue-empty {
    padding: 70px 20px;
  }
  .queue-pagination {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 16px;
    padding: 6px 0;
    margin-left: auto;
  }
  @media (max-width: 1100px) {
    .queue-filters {
      grid-template-columns: repeat(3, minmax(120px, 1fr));
    }
    .queue-search {
      grid-column: span 1;
    }
  }
  @media (max-width: 750px) {
    .queue-filters {
      grid-template-columns: repeat(2, minmax(100px, 1fr));
    }
    .job-heading {
      flex-wrap: wrap;
    }
    .job-identity {
      flex-basis: 60%;
    }
    .job-identity p {
      flex-wrap: wrap;
      gap: 3px 14px;
    }
  }
</style>
