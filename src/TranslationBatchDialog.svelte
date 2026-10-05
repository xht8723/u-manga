<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { BookA, Check, RotateCcw, ShieldCheck } from 'lucide-svelte';
  import Modal from './Modal.svelte';
  import GlossaryModal from './GlossaryModal.svelte';
  import RequirementHint from './RequirementHint.svelte';
  import { actionReadiness } from './action-readiness.svelte';
  import { latestCheck, type CheckState } from './latest-check';
  import { batchCounts } from './batch-translation';
  import { submitTranslationBatch } from './job-events';
  import { call } from './bridge';
  import { effectiveSettings } from './book-state';
  import { t, tr, locale, number, uiError, type UiText } from './i18n';
  import type {
    Book,
    Job,
    Settings,
    TranslationBatchMode,
    TranslationBatchPreview,
    TranslationBatchResult,
  } from './types';

  let {
    book,
    chapterId,
    settings,
    jobs,
    onclose,
    onsettings,
    onworking,
    onsubmitted,
    onbook,
  }: {
    book: Book;
    chapterId: string | null;
    settings: Settings;
    jobs: Job[];
    onclose: () => void;
    onsettings: () => void;
    onworking: (working: boolean) => void;
    onsubmitted: (result: TranslationBatchResult) => void;
    onbook: (updated: Book) => void;
  } = $props();
  let mode = $state<TranslationBatchMode>('skip_translated');
  let working = $state(false),
    error = $state<UiText>(''),
    epoch = $state(0);
  let glossaryOpen = $state(false),
    glossaryLoading = $state(false);
  let glossaryButton = $state<HTMLButtonElement>();
  let preview = $state<CheckState<TranslationBatchPreview>>({
    value: null,
    fresh: false,
    pending: true,
    showActivity: false,
    error: '',
  });
  const library = untrack(() => settings.libraryDirectory);
  const check = latestCheck(
    (input: { path: string; chapterId: string | null }) => call('translation_batch_preview', input),
    (next) => {
      preview = next;
    },
  );
  const busyPages = $derived(
    jobs
      .filter(
        (j) =>
          j.project === book.path &&
          (j.stopping || ['queued', 'running', 'paused'].includes(j.status)),
      )
      .map((j) => j.pageId)
      .sort()
      .join(','),
  );
  $effect(() => {
    busyPages;
    epoch;
    book.revision;
    return check.start({ path: book.path, chapterId });
  });
  const readiness = actionReadiness(() => ({
    settings: {
      ...settings,
      translation: effectiveSettings($state.snapshot(book), $state.snapshot(settings)),
    },
    path: book.path,
    actions: ['translate'],
  }));
  const counts = $derived(batchCounts(preview.value, mode));
  const validLibrary = $derived(settings.libraryDirectory === library);
  const validBook = $derived(!preview.value || preview.value.bookId === book.id);
  const canSubmit = $derived(
    preview.fresh &&
      validLibrary &&
      validBook &&
      readiness.ready('translate') &&
      counts.eligible > 0 &&
      !working,
  );
  const title = $derived(
    chapterId ? tr('Translate chapter', $locale) : tr('Translate full book', $locale),
  );
  const scopeTitle = $derived(
    preview.value?.chapterTitle ??
      book.chapters.find((c) => c.id === chapterId)?.title ??
      book.metadata.title,
  );
  const status = $derived(
    (working ? t('batch.adding', $locale) : '') ||
      error ||
      preview.error ||
      (!validLibrary
        ? t('batch.library_changed', $locale)
        : !validBook
          ? t('batch.book_changed', $locale)
          : preview.showActivity
            ? t('batch.checking', $locale)
            : preview.fresh && !counts.eligible
              ? t('batch.no_eligible', $locale)
              : ''),
  );
  onMount(() => {
    const refresh = () => {
      epoch++;
    };
    window.addEventListener('focus', refresh);
    window.addEventListener('umanga-requirements-changed', refresh);
    return () => {
      check.cancel();
      window.removeEventListener('focus', refresh);
      window.removeEventListener('umanga-requirements-changed', refresh);
    };
  });
  async function submit() {
    if (!canSubmit || !preview.value || glossaryOpen || glossaryLoading) return;
    const snapshot = $state.snapshot(preview.value);
    const selected = mode;
    working = true;
    error = '';
    onworking(true);
    try {
      const result = await submitTranslationBatch(
        book.path,
        book.id,
        chapterId,
        snapshot.pages.map(({ id, revision }) => ({ id, revision })),
        selected,
      );
      onsubmitted(result);
    } catch (e) {
      error = uiError(e);
      epoch++;
    } finally {
      working = false;
      onworking(false);
    }
  }
  async function openGlossary() {
    if (working || glossaryLoading) return;
    glossaryLoading = true;
    error = '';
    try {
      const saved = await call('book_open', { path: book.path, glossary: true });
      if (saved.revision >= book.revision) onbook(saved);
      glossaryOpen = true;
    } catch (e) {
      error = uiError(e);
    } finally {
      glossaryLoading = false;
    }
  }
  function closeGlossary() {
    glossaryOpen = false;
    epoch++;
    window.dispatchEvent(new Event('umanga-requirements-changed'));
  }
</script>

<Modal
  {title}
  inactive={glossaryOpen}
  className="batch-translation-modal"
  closeDisabled={working}
  onclose={() => {
    if (!working) onclose();
  }}
>
  <div class="batch-dialog">
    <div class="batch-target">
      <h3>{scopeTitle}</h3>
      {#if chapterId}<small>{preview.value?.bookTitle ?? book.metadata.title}</small>{/if}
    </div>
    <fieldset class="batch-choices" disabled={working}>
      <legend class="sr-only">{t('batch.choice', $locale)}</legend>
      <label class:chosen={mode === 'skip_translated'}>
        <input type="radio" name="batch-mode" value="skip_translated" bind:group={mode} />
        <ShieldCheck size={22} aria-hidden="true" />
        <span
          ><strong>{t('batch.skip', $locale)}</strong><small
            >{t('batch.skip_description', $locale)}</small
          ></span
        >
        <span class="choice-check" aria-hidden="true"
          >{#if mode === 'skip_translated'}<Check size={16} />{/if}</span
        >
      </label>
      <label class:chosen={mode === 'replace'}>
        <input type="radio" name="batch-mode" value="replace" bind:group={mode} />
        <RotateCcw size={22} aria-hidden="true" />
        <span
          ><strong>{t('batch.replace', $locale)}</strong><small
            >{t('batch.replace_description', $locale)}</small
          ></span
        >
        <span class="choice-check" aria-hidden="true"
          >{#if mode === 'replace'}<Check size={16} />{/if}</span
        >
      </label>
    </fieldset>
    <dl class="batch-counts" aria-busy={!preview.fresh}>
      <div>
        <dt>{t('batch.to_process', $locale)}</dt>
        <dd>{preview.value ? number(counts.eligible, $locale) : '—'}</dd>
      </div>
      <div>
        <dt>{t(mode === 'replace' ? 'batch.to_replace' : 'batch.kept', $locale)}</dt>
        <dd>
          {preview.value
            ? number(mode === 'replace' ? counts.replacing : counts.skipped, $locale)
            : '—'}
        </dd>
      </div>
      <div>
        <dt>{t('batch.busy', $locale)}</dt>
        <dd>{preview.value ? number(counts.busy, $locale) : '—'}</dd>
      </div>
      <div>
        <dt>{t('organizer.omitted', $locale)}</dt>
        <dd>{preview.value ? number(counts.omitted, $locale) : '—'}</dd>
      </div>
    </dl>
    {#if mode === 'replace'}<div class="batch-notice">
        <p>{t('batch.replace_notice', $locale)}</p>
      </div>{/if}
    <div
      class="batch-status"
      class:empty={!status && !readiness.hint('translate')}
      aria-live="polite"
    >
      {#if status}<p class:error-text={!!error || !!preview.error}>{tr(status, $locale)}</p>{/if}
      <RequirementHint
        reason={readiness.hint('translate')}
        disabled={working}
        onsettings={() => {
          if (!working) onsettings();
        }}
      />
    </div>
  </div>
  {#snippet footer()}
    <button bind:this={glossaryButton} disabled={working || glossaryLoading} onclick={openGlossary}
      ><BookA size={16} aria-hidden="true" />{tr('Glossary', $locale)}</button
    >
    <span class="spacer"></span>
    <button disabled={working} onclick={onclose}>{tr('Cancel', $locale)}</button>
    <button class="primary" disabled={!canSubmit || glossaryLoading} onclick={submit}
      >{t(working ? 'batch.adding' : 'batch.submit', $locale)}</button
    >
  {/snippet}
</Modal>
{#if glossaryOpen}<GlossaryModal
    {book}
    {settings}
    portal
    returnFocus={glossaryButton}
    onclose={closeGlossary}
    onsaved={onbook}
  />{/if}

<style>
  :global(.library-modal.batch-translation-modal) {
    width: min(620px, 94vw);
    max-height: 90dvh;
  }
  :global(.batch-translation-modal > header),
  :global(.batch-translation-modal > footer) {
    flex: none;
  }
  :global(.batch-translation-modal > .modal-body) {
    flex: 1;
    scrollbar-gutter: stable;
  }
  .batch-dialog {
    display: flex;
    flex-direction: column;
    gap: 20px;
  }
  .batch-target h3 {
    font-size: 1.1em;
    line-height: 1.4;
    overflow-wrap: anywhere;
    margin: 0;
  }
  .batch-target small {
    display: block;
    margin-top: 5px;
    overflow-wrap: anywhere;
  }
  .batch-choices {
    border: 0;
    padding: 0;
    margin: 0;
    display: grid;
    gap: 12px;
    min-width: 0;
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
  .batch-choices label {
    position: relative;
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 14px;
    margin: 0;
    padding: 18px;
    border: 1px solid var(--border);
    border-radius: 10px;
    cursor: pointer;
    font-size: 1em;
    background: var(--panel);
  }
  .batch-choices label:hover {
    background: var(--raised);
  }
  .batch-choices label.chosen {
    border-color: var(--accent);
    background: var(--selection);
  }
  .batch-choices label:focus-within {
    outline: 2px solid var(--focus);
    outline-offset: 2px;
  }
  .batch-choices input {
    position: absolute;
    width: 1px;
    height: 1px;
    opacity: 0;
    margin: 0;
  }
  .batch-choices label > :global(svg) {
    flex: none;
    color: var(--muted);
  }
  .batch-choices label > span:not(.choice-check) {
    flex: 1;
    min-width: 0;
  }
  .batch-choices strong {
    display: block;
    font-size: 1.06em;
    line-height: 1.45;
  }
  .batch-choices small {
    display: block;
    margin-top: 5px;
    font-weight: 400;
    line-height: 1.5;
  }
  .choice-check {
    width: 22px;
    height: 22px;
    flex: none;
    display: grid;
    place-items: center;
    color: var(--accent);
  }
  .batch-counts {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 12px;
    margin: 0;
    padding: 14px 0;
    border-block: 1px solid var(--border);
  }
  .batch-counts dt {
    color: var(--muted);
    font-size: 0.85em;
    line-height: 1.5;
  }
  .batch-counts dd {
    margin: 5px 0 0;
    font-size: 1.45em;
    font-weight: 650;
    font-variant-numeric: tabular-nums;
  }
  .batch-notice p {
    margin: 0;
    font-size: 0.9em;
    line-height: 1.6;
    color: var(--muted);
  }
  .batch-status.empty {
    display: none;
  }
  .batch-status p {
    margin: 0;
    font-size: 0.9em;
    line-height: 1.5;
  }
  @media (max-width: 520px) {
    .batch-counts {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
    .batch-choices label {
      padding: 14px;
      gap: 10px;
    }
    .batch-dialog {
      gap: 16px;
    }
  }
</style>
