<script lang="ts">
  import { randomUuid } from './browser-id';
  import { uiError, type UiText, t, tr, locale } from './i18n';
  import { untrack } from 'svelte';
  import { ImagePlus, GripVertical } from 'lucide-svelte';
  import { sweepSelection } from './page-selection';
  import Modal from './Modal.svelte';
  import Thumbnail from './Thumbnail.svelte';
  import { call } from './bridge';
  import { chooseSources, chooseChapterImages, confirmDelete } from './files';
  import { chapter } from './book-state';
  import type { Book, Chapter, Page, ImportPreview } from './types';
  let {
    book,
    initial,
    onclose,
    onapply,
    oncreate,
    onback,
    automatic = $bindable(true),
    scale = 1,
  }: {
    book?: Book;
    initial?: ImportPreview;
    onclose: () => void;
    onapply?: (book: Book) => void;
    oncreate?: (draft: ImportPreview) => Promise<void>;
    onback?: (draft: ImportPreview) => void;
    automatic?: boolean;
    scale?: number;
  } = $props();
  let chapters = $state<Chapter[]>(
      untrack(() => structuredClone($state.snapshot(book?.chapters || initial?.chapters || []))),
    ),
    added = $state<Page[]>(untrack(() => structuredClone($state.snapshot(initial?.pages || [])))),
    omittedPageIds = $state<string[]>(
      untrack(() => [...(book?.omittedPageIds || initial?.omittedPageIds || [])]),
    ),
    selected = $state<string[]>([]),
    current = $state(untrack(() => (book?.chapters || initial?.chapters)?.[0]?.id || ''));
  let warnings = $state<UiText[]>(untrack(() => [...(initial?.warnings || [])]));
  const baseChapters = untrack(() => JSON.stringify(book?.chapters || []));
  const baseOmissions = untrack(() => [...(book?.omittedPageIds || [])]);
  function draft(): ImportPreview {
    const retained = new Set(chapters.flatMap((c) => c.pageIds));
    return {
      chapters: $state.snapshot(chapters),
      omittedPageIds: omittedPageIds.filter((id) => retained.has(id)),
      pages: $state.snapshot(added).filter((p) => retained.has(p.id)),
      warnings: $state.snapshot(warnings),
    };
  }
  type History = { chapters: Chapter[]; omittedPageIds: string[] };
  let undo = $state<History[]>([]),
    redo = $state<History[]>([]),
    error = $state<UiText>(''),
    working = $state(false),
    committing = $state(false),
    scanId = '',
    dragChapter = '',
    dragPage = '';
  let destination = $state(''),
    moveDialog = $state(false),
    pageScroll = $state(0),
    pageHeight = $state(500),
    pageWidth = $state(800);
  let active = $derived(chapters.find((c) => c.id === current));
  let omitted = $derived(new Set(omittedPageIds));
  let selectedOmitted = $derived(!!selected.length && selected.every((id) => omitted.has(id)));
  let pageViewport = $state<HTMLDivElement>();
  $effect(() => {
    current;
    if (pageViewport) {
      pageViewport.scrollTop = 0;
      pageScroll = 0;
    }
  });
  let uiScale = $derived(Math.max(1, scale));
  let pageColumns = $derived(Math.max(1, Math.floor((pageWidth + 12) / (132 * uiScale + 12))));
  let pageRowHeight = $derived(Math.round(228 * uiScale));
  let pageStart = $derived(Math.max(0, Math.floor(pageScroll / pageRowHeight) - 2) * pageColumns);
  let pageEnd = $derived(pageStart + (Math.ceil(pageHeight / pageRowHeight) + 4) * pageColumns);
  function remember() {
    undo.push(historySnapshot());
    redo = [];
  }
  function historySnapshot(): History {
    return structuredClone({
      chapters: $state.snapshot(chapters),
      omittedPageIds: $state.snapshot(omittedPageIds),
    });
  }
  function toggleOmission() {
    if (working || !selected.length) return;
    remember();
    omittedPageIds = selectedOmitted
      ? omittedPageIds.filter((id) => !selected.includes(id))
      : [...new Set([...omittedPageIds, ...selected])];
  }
  function history(back: boolean) {
    const from = back ? undo : redo,
      to = back ? redo : undo;
    const prev = from.pop();
    if (prev) {
      to.push(historySnapshot());
      chapters = prev.chapters;
      omittedPageIds = prev.omittedPageIds;
      if (!chapters.some((c) => c.id === current)) current = chapters[0]?.id || '';
      selected = [];
    }
  }
  function reorder(from: number, to: number) {
    if (from === to || from < 0 || to < 0 || to >= chapters.length) return;
    remember();
    const [c] = chapters.splice(from, 1);
    chapters.splice(to, 0, c);
  }
  function reorderPage(from: string, to: string) {
    if (!active || from === to) return;
    const a = active.pageIds.indexOf(from),
      b = active.pageIds.indexOf(to);
    if (a < 0 || b < 0) return;
    remember();
    active.pageIds.splice(a, 1);
    active.pageIds.splice(b, 0, from);
  }
  function move() {
    if (!active || destination === current) return;
    const target = chapters.find((c) => c.id === destination);
    if (!target) return;
    remember();
    target.pageIds.push(...active.pageIds.filter((id) => selected.includes(id)));
    active.pageIds = active.pageIds.filter((id) => !selected.includes(id));
    target.read = false;
    selected = [];
    moveDialog = false;
  }
  function split() {
    if (!active || selected.length !== 1) return;
    const at = active.pageIds.indexOf(selected[0]);
    if (at < 1) return;
    remember();
    const next = chapter(`${active.title} · 2`);
    next.pageIds = active.pageIds.splice(at);
    next.read = active.read;
    chapters.splice(chapters.indexOf(active) + 1, 0, next);
    selected = [];
  }
  function merge() {
    if (!active) return;
    const i = chapters.indexOf(active);
    if (i === chapters.length - 1) return;
    remember();
    const next = chapters.splice(i + 1, 1)[0];
    active.pageIds.push(...next.pageIds);
    active.read = active.read && next.read;
  }
  async function removeChapter(c: Chapter) {
    remember();
    chapters = chapters.filter((x) => x.id !== c.id);
    omittedPageIds = omittedPageIds.filter((id) => !c.pageIds.includes(id));
    current = chapters[0]?.id || '';
    selected = [];
  }
  async function removePages() {
    if (!active || !selected.length) return;
    remember();
    active.pageIds = active.pageIds.filter((id) => !selected.includes(id));
    omittedPageIds = omittedPageIds.filter((id) => !selected.includes(id));
    selected = [];
  }
  async function sources(target?: Chapter) {
    if (working) return;
    working = true;
    error = '';
    scanId = randomUuid();
    try {
      const paths = target ? await chooseChapterImages(target.title) : await chooseSources(true);
      if (!paths.length) return;
      const p = await call('import_scan', {
        id: scanId,
        paths,
        automatic: target ? false : automatic,
      });
      const identities = new Set<string>();
      const existing = book ? await call('book_sources', { path: book.path }) : [];
      const retained = new Set(chapters.flatMap((c) => c.pageIds));
      for (const page of [...existing, ...added].filter((p) => retained.has(p.id)))
        identities.add(JSON.stringify(page.source));
      const fresh = p.pages.filter((page) => {
        const k = JSON.stringify(page.source);
        if (identities.has(k)) {
          p.warnings.push(`Duplicate skipped: ${page.name}`);
          return false;
        }
        identities.add(k);
        return true;
      });
      const ids = new Set(fresh.map((p) => p.id));
      remember();
      added.push(...fresh);
      if (target) {
        target.pageIds.push(...fresh.map((p) => p.id));
        if (fresh.length) target.read = false;
        current = target.id;
        selected = [];
      } else
        chapters.push(
          ...p.chapters
            .map((c) => ({ ...c, pageIds: c.pageIds.filter((id) => ids.has(id)) }))
            .filter(
              (c) =>
                c.pageIds.length ||
                !p.chapters.find((original) => original.id === c.id)?.pageIds.length,
            ),
        );
      current ||= chapters[0]?.id || '';
      warnings.push(...p.warnings);
    } catch (e) {
      error = uiError(e);
    } finally {
      committing = false;
      working = false;
    }
  }
  async function apply() {
    if (!book) {
      committing = true;
      working = true;
      error = '';
      try {
        await oncreate?.(draft());
      } catch (e) {
        error = uiError(e);
      } finally {
        committing = false;
        working = false;
      }
      return;
    }
    const retained = new Set(chapters.flatMap((c) => c.pageIds));
    const deleted = book.chapters.flatMap((c) => c.pageIds).filter((id) => !retained.has(id));
    const removedChapters = book.chapters.filter(
      (c) =>
        !chapters.some((next) => next.id === c.id) &&
        (!c.pageIds.length || c.pageIds.some((id) => deleted.includes(id))),
    );
    const names = [
      ...removedChapters.map((c) => c.title),
      ...book.chapters
        .filter((c) => !removedChapters.includes(c))
        .flatMap((c) =>
          c.pageIds.flatMap((id, i) =>
            deleted.includes(id) ? [`${c.title} · Page ${i + 1}`] : [],
          ),
        ),
    ];
    if (names.length && !(await confirmDelete(names))) return;
    committing = true;
    working = true;
    try {
      onapply?.(
        !added.length && JSON.stringify($state.snapshot(chapters)) === baseChapters
          ? await call('book_omissions_update', { path: book.path, base: baseOmissions, omittedPageIds: $state.snapshot(omittedPageIds) })
          : await call('book_organize', {
          path: book.path,
          expected: book.revision,
          chapters: $state.snapshot(chapters),
          added: $state.snapshot(added).filter((p) => retained.has(p.id)),
          omittedPageIds: omittedPageIds.filter((id) => retained.has(id)),
        }),
      );
    } catch (e) {
      error = uiError(e);
    } finally {
      committing = false;
      working = false;
    }
  }
  async function close() {
    if (committing) return;
    if (working) await call('import_cancel', { id: scanId });
    else onclose();
  }
</script>

<Modal
  title={tr(book ? 'Manage chapters' : 'Add book · Chapters', $locale)}
  wide
  stretch
  inactive={moveDialog}
  onclose={() => void close()}
>
  <fieldset class="organizer-controls chapter-organizer" disabled={working}>
    <div class="row">
      <label class="check"
        ><input title={t('m_d526b52fc612', $locale)} type="checkbox" bind:checked={automatic} />{t(
          'm_b9e78af291c5',
          $locale,
        )}</label
      ><span class="spacer"></span><button
        disabled={working}
        title={t('m_4736ddcf4129', $locale)}
        onclick={() => sources()}>{t('m_5bbfc5a69b25', $locale)}</button
      ><button
        disabled={working}
        title={t('m_7f1d2fb4155f', $locale)}
        onclick={() => {
          remember();
          const c = chapter();
          chapters.push(c);
          current = c.id;
          selected = [];
        }}>{t('m_e180dea15a06', $locale)}</button
      >
    </div>
    <div class="organizer-layout">
      <div class="chapter-organizer-list">
        {#each chapters as c, i (c.id)}<div
            class="organizer-chapter"
            class:current={c.id === current}
            draggable={!working}
            role="group"
            aria-label={c.title}
            ondragstart={() => (dragChapter = c.id)}
            ondragover={(e) => e.preventDefault()}
            ondrop={(e) => {
              e.preventDefault();
              reorder(
                chapters.findIndex((c) => c.id === dragChapter),
                i,
              );
            }}
          >
            <div class="chapter-heading">
              <button
                class="chapter-select"
                title={tr(`Manage pages in ${c.title}`, $locale)}
                onclick={() => {
                  current = c.id;
                  selected = [];
                  pageScroll = 0;
                }}
                >{i + 1} · {c.title}<small>{c.pageIds.length} {t('m_bfa062de040f', $locale)}</small
                ></button
              ><button
                class="chapter-add-files"
                disabled={working}
                title={tr(`Add image files to ${c.title}`, $locale)}
                aria-label={tr(`Add files to ${c.title}`, $locale)}
                onclick={() => sources(c)}><ImagePlus size={18} /></button
              >
            </div>
            <div class="row">
              <button
                disabled={i === 0 || working}
                title={t('m_8fa059f6a7e3', $locale)}
                aria-label={tr(`Move ${c.title} up`, $locale)}
                onclick={() => reorder(i, i - 1)}>↑</button
              ><button
                disabled={i === chapters.length - 1 || working}
                title={t('m_b29326f10de8', $locale)}
                aria-label={tr(`Move ${c.title} down`, $locale)}
                onclick={() => reorder(i, i + 1)}>↓</button
              ><button
                disabled={working}
                onclick={() => removeChapter(c)}
                title={t('m_c4463b1b16f2', $locale)}
                aria-label={tr(`Delete ${c.title}`, $locale)}>✕</button
              >
            </div>
          </div>{/each}
      </div>
      <div class="organizer-pages">
        {#if active}<label
            >{t('m_2ffc2128bb47', $locale)}<input
              value={active.title}
              onchange={(e) => {
                remember();
                active!.title = e.currentTarget.value;
              }}
            /></label
          >
          <div class="row">
            <button
              disabled={chapters.indexOf(active) === chapters.length - 1 || working}
              title={t('m_6718578a52c9', $locale)}
              onclick={merge}>{t('m_2e0a6cdd6191', $locale)}</button
            ><button
              disabled={selected.length !== 1 || active.pageIds.indexOf(selected[0]) < 1 || working}
              title={t('m_4e5222e4e93e', $locale)}
              onclick={split}>{t('m_86797cd720bf', $locale)}</button
            >
          </div>
          <div class="row">
            <label class="check"
              ><input
                type="checkbox"
                checked={!!active.pageIds.length && selected.length === active.pageIds.length}
                onchange={(e) => (selected = e.currentTarget.checked ? [...active!.pageIds] : [])}
              />{t('m_a52ace420f21', $locale)}</label
            ><span>{selected.length} {t('m_d7cbbb688b2e', $locale)}</span><span class="spacer"
            ></span><button
              disabled={!selected.length || working}
              title={t('organizer.omission_hint', $locale)}
              onclick={toggleOmission}
              >{t(selectedOmitted ? 'organizer.include' : 'organizer.omit', $locale)}</button
            ><button
              disabled={!selected.length || chapters.length < 2 || working}
              title={tr(
                chapters.length < 2
                  ? 'Add another chapter to move pages'
                  : 'Choose a chapter for the selected pages',
                $locale,
              )}
              onclick={() => {
                destination = '';
                moveDialog = true;
              }}>{t('m_6ecc3df6bffd', $locale)}</button
            ><button
              disabled={!selected.length || working}
              title={t('m_391fa5300d5b', $locale)}
              onclick={removePages}>{t('m_e2d0a54968ea', $locale)}</button
            >
          </div>
          <div
            class="organizer-page-list"
            use:sweepSelection={{ selected: () => selected, change: (ids) => selected = ids, enabled: () => !working && !moveDialog, identity: () => current }}
            bind:clientWidth={pageWidth}
            bind:this={pageViewport}
            bind:clientHeight={pageHeight}
            onscroll={(e) => (pageScroll = e.currentTarget.scrollTop)}
          >
            <div
              style:height={`${Math.ceil(active.pageIds.length / pageColumns) * pageRowHeight}px`}
              class="grid-spacer"
            >
              <div
                class="organizer-page-grid"
                style:grid-template-columns={`repeat(${pageColumns}, minmax(0, 1fr))`}
                style:transform={`translateY(${Math.floor(pageStart / pageColumns) * pageRowHeight}px)`}
              >
                {#each active.pageIds.slice(pageStart, pageEnd) as id, j (id)}{@const i =
                    pageStart + j}
                  <div
                    class="organizer-page"
                    class:selected={selected.includes(id)}
                    data-selection-page={id}
                    style:height={`${pageRowHeight - 12}px`}
                    role="group"
                    aria-label={tr(`Page ${i + 1}`, $locale)}
                    ondragover={(e) => e.preventDefault()}
                    ondrop={(e) => {
                      e.preventDefault();
                      reorderPage(dragPage, id);
                    }}
                  >
                    <label class="page-selection"
                      ><input
                        type="checkbox"
                        aria-label={tr(`Select page ${i + 1}`, $locale)}
                        checked={selected.includes(id)}
                        onchange={(e) =>
                          (selected = e.currentTarget.checked
                            ? [...selected, id]
                            : selected.filter((x) => x !== id))}
                      /><span>{t('m_0a30a815d67d', $locale)} {i + 1}</span></label
                    >
                    <div class="organizer-thumbnail">
                      <Thumbnail
                        path={book?.path || ''}
                        pageId={id}
                        source={added.find((p) => p.id === id)?.source}
                      />
                      {#if omitted.has(id)}<span
                          class="omitted-badge"
                          title={t('organizer.omission_hint', $locale)}
                          >{t('organizer.omitted', $locale)}</span
                        >{/if}
                    </div>
                    <div class="page-order-actions">
                      <button
                        class="page-reorder-grip"
                        data-reorder-handle
                        draggable={!working}
                        disabled={working}
                        title={t('organizer.reorderDrag', $locale)}
                        aria-label={t('organizer.reorderDrag', $locale)}
                        ondragstart={(e) => {
                          dragPage = id;
                          if (e.dataTransfer) { e.dataTransfer.effectAllowed = 'move'; e.dataTransfer.setData('text/plain', id); }
                        }}
                        ondragend={() => (dragPage = '')}
                      ><GripVertical size={16} /></button>
                      <button
                        disabled={i === 0 || working}
                        title={t('m_8714e92183fb', $locale)}
                        aria-label={t('m_8714e92183fb', $locale)}
                        onclick={() => reorderPage(id, active!.pageIds[i - 1])}>←</button
                      ><button
                        disabled={i === active.pageIds.length - 1 || working}
                        title={t('m_31b459f22510', $locale)}
                        aria-label={t('m_31b459f22510', $locale)}
                        onclick={() => reorderPage(id, active!.pageIds[i + 1])}>→</button
                      >
                    </div>
                  </div>{/each}
              </div>
            </div>
          </div>{:else}<div class="empty">{t('m_e2e336e6759a', $locale)}</div>{/if}
      </div>
    </div>
  </fieldset>
  {#each warnings as warning}<p class="review-note">{tr(warning, $locale)}</p>{/each}
  {#if error}<p class="error-text" role="alert">{tr(error, $locale)}</p>{/if}
  {#snippet footer()}<button
      disabled={!undo.length || working}
      title={t('m_9fbae071f93d', $locale)}
      onclick={() => history(true)}>{t('m_a8283ade3185', $locale)}</button
    >
    <button
      disabled={!redo.length || working}
      title={t('m_de3cc96de7fa', $locale)}
      onclick={() => history(false)}>{t('m_74273989b096', $locale)}</button
    ><span
      >{working
        ? committing
          ? t('m_3329a9bb48b9', $locale)
          : t('m_38d96da6e8b2', $locale)
        : tr(`${chapters.length} chapters`, $locale)}</span
    ><span class="spacer"></span><button disabled={committing} onclick={() => void close()}
      >{working ? t('m_298360e0b166', $locale) : t('m_19766ed6ccb2', $locale)}</button
    >{#if onback}<button
        disabled={working}
        title={t('m_c1fa17ef3176', $locale)}
        onclick={() => onback?.(draft())}>{t('m_76900f1bfd16', $locale)}</button
      >{/if}<button
      class="primary"
      disabled={working || chapters.some((c) => !c.title.trim())}
      onclick={apply}>{book ? t('m_31e392d1c037', $locale) : t('m_8696e459b40f', $locale)}</button
    >{/snippet}
</Modal>

{#if moveDialog}
  <Modal title={t('m_09702fa26294', $locale)} onclose={() => (moveDialog = false)}>
    <p class="move-summary">
      {tr(`Move ${selected.length} pages from ${active?.title ?? ''}`, $locale)}
    </p>
    <fieldset class="move-destinations">
      <legend>{t('m_c6b40bacdb28', $locale)}</legend>
      {#each chapters.filter((c) => c.id !== current) as c (c.id)}
        <label class="move-destination" class:chosen={destination === c.id}>
          <input type="radio" name="move-destination" value={c.id} bind:group={destination} />
          <span>{c.title}<small>{c.pageIds.length} {t('m_bfa062de040f', $locale)}</small></span>
        </label>
      {/each}
    </fieldset>
    {#snippet footer()}
      <button onclick={() => (moveDialog = false)}>{t('m_19766ed6ccb2', $locale)}</button>
      <button class="primary" disabled={!destination} onclick={move}
        >{t('m_09702fa26294', $locale)}</button
      >
    {/snippet}
  </Modal>
{/if}
