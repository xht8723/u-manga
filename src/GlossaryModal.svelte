<script lang="ts">
  import { randomUuid } from './browser-id';
  import { untrack, tick } from 'svelte';
  import {
    BookA,
    Search,
    Plus,
    Trash2,
    Upload,
    Download,
    ChevronLeft,
    ChevronRight,
  } from 'lucide-svelte';
  import { save } from '@tauri-apps/plugin-dialog';
  import Modal from './Modal.svelte';
  import ToggleSwitch from './ToggleSwitch.svelte';
  import ComboBox from './ComboBox.svelte';
  import { call, native } from './bridge';
  import { tr, locale, uiError, type UiText } from './i18n';
  import { effectiveSettings } from './book-state';
  import { languageName } from './setup-state';
  import { entryErrors, mergeTerms, normalizeGlossary, MAX_GLOSSARY_FILE_BYTES } from './glossary';
  import { syncGlossary } from './glossary-sync';
  import type { Book, Settings, GlossaryEntry } from './types';
  let {
    book,
    settings,
    onclose,
    onsaved,
    portal = false,
    returnFocus,
  }: {
    book: Book;
    settings: Settings;
    onclose: () => void;
    onsaved: (book: Book) => void;
    portal?: boolean;
    returnFocus?: HTMLElement;
  } = $props();
  let baseline = $state(untrack(() => structuredClone($state.snapshot(book.glossary))));
  const targetPath = untrack(() => book.path);
  let draft = $state(untrack(() => structuredClone($state.snapshot(baseline))));
  let ids = $state<string[]>(untrack(() => draft.entries.map(() => randomUuid())));
  let selected = $state<string[]>([]),
    search = $state(''),
    page = $state(0),
    working = $state(false),
    error = $state<UiText>('');
  let confirmClose = $state(false),
    refreshConflict = $state(false),
    incoming = $state<GlossaryEntry[] | null>(null),
    importName = $state(''),
    replace = $state(false),
    importPage = $state(0);
  let exportFormat = $state('csv'),
    input = $state<HTMLInputElement>(null!);
  let effective = $derived(effectiveSettings($state.snapshot(book), $state.snapshot(settings)));
  let service = $derived(settings.providers.find((p) => p.id === effective.providerId)?.service);
  let dirty = $derived(JSON.stringify(draft) !== JSON.stringify(baseline));
  let errors = $derived(entryErrors(draft.entries));
  let valid = $derived(!errors.some(Boolean));
  let filtered = $derived(
    draft.entries
      .map((e, i) => ({ e, i, id: ids[i] }))
      .filter(({ e }) =>
        (e.source + '\n' + e.target).toLocaleLowerCase().includes(search.toLocaleLowerCase()),
      ),
  );
  let pages = $derived(Math.max(1, Math.ceil(filtered.length / 50)));
  let visible = $derived(filtered.slice(page * 50, (page + 1) * 50));
  let preview = $derived(incoming ? mergeTerms(draft.entries, incoming, replace) : null);
  let importPages = $derived(Math.max(1, Math.ceil((incoming?.length || 0) / 50)));
  $effect(() => {
    if (page >= pages) page = pages - 1;
  });
  function refreshSaved() {
    if (book.path !== targetPath) return;
    const result = syncGlossary(
      $state.snapshot(baseline),
      $state.snapshot(draft),
      $state.snapshot(book.glossary),
    );
    if (!result) return;
    refreshConflict = result.conflict;
    if (result.conflict) return;
    const known = new Map(draft.entries.map((e, i) => [e.source, ids[i]]));
    ids =
      result.appended === null
        ? result.draft.entries.map((e) => known.get(e.source) || randomUuid())
        : [...ids, ...Array.from({ length: result.appended }, () => randomUuid())];
    baseline = result.baseline;
    draft = result.draft;
    selected = selected.filter((id) => ids.includes(id));
  }
  $effect(() => {
    book.glossary;
    dirty;
    if (!working) untrack(refreshSaved);
  });
  function close() {
    if (working) return;
    if (incoming) {
      incoming = null;
      return;
    }
    if (dirty) confirmClose = true;
    else onclose();
  }
  async function add() {
    search = '';
    const id = randomUuid();
    ids = [...ids, id];
    draft.entries.push({ source: '', target: '' });
    page = Math.floor((draft.entries.length - 1) / 50);
    await tick();
    document.getElementById('glossary-source-' + id)?.focus();
  }
  function remove(chosen: string[]) {
    const keep = draft.entries
      .map((e, i) => ({ e, id: ids[i] }))
      .filter((r) => !chosen.includes(r.id));
    draft.entries = keep.map((r) => r.e);
    ids = keep.map((r) => r.id);
    selected = selected.filter((id) => !chosen.includes(id));
  }
  async function readFile(file?: File) {
    if (!file) return;
    working = true;
    confirmClose = false;
    error = '';
    try {
      if (file.size > MAX_GLOSSARY_FILE_BYTES)
        throw Error('Glossary file exceeds the encoded size limit.');
      const text = new TextDecoder('utf-8', { fatal: true }).decode(await file.arrayBuffer());
      const result = await call('glossary_parse', {
        text,
        format: file.name.split('.').pop()?.toLowerCase() || '',
      });
      incoming = result;
      importName = file.name;
      replace = false;
      importPage = 0;
    } catch (e) {
      error = uiError(e);
    } finally {
      working = false;
      input.value = '';
    }
  }
  function merge() {
    if (!preview) return;
    const known = new Map(draft.entries.map((e, i) => [e.source.trim(), ids[i]]));
    draft.automaticSources = draft.automaticSources.filter(
      (s) => !incoming?.some((e) => e.source.trim() === s),
    );
    draft.entries = preview.entries;
    ids = draft.entries.map((e) => known.get(e.source) || randomUuid());
    incoming = null;
    selected = [];
    search = '';
    page = 0;
  }
  async function commit() {
    if (working || !valid) return;
    refreshSaved();
    if (refreshConflict) return;
    working = true;
    confirmClose = false;
    error = '';
    try {
      const value = normalizeGlossary($state.snapshot(draft));
      const saved = await call('book_glossary_save', {
        path: targetPath,
        expected: baseline.revision,
        glossary: value,
      });
      onsaved(saved);
      onclose();
    } catch (e) {
      error = uiError(e);
    } finally {
      working = false;
    }
  }
  async function exportFile() {
    if (working || !valid) return;
    working = true;
    confirmClose = false;
    error = '';
    try {
      const entries = normalizeGlossary($state.snapshot(draft)).entries;
      const filename =
        book.metadata.title.replace(/[<>:"/\\|?*]/g, '_') + '-glossary.' + exportFormat;
      const destination = native
        ? await save({
            title: tr('Export glossary'),
            defaultPath: filename,
            filters: [{ name: exportFormat.toUpperCase(), extensions: [exportFormat] }],
          })
        : filename;
      if (destination)
        await call('glossary_export', {
          path: targetPath,
          destination,
          entries,
          format: exportFormat,
        });
    } catch (e) {
      error = uiError(e);
    } finally {
      working = false;
    }
  }
</script>

<Modal
  title={tr('Glossary', $locale)}
  wide
  stretch
  {portal}
  {returnFocus}
  closeDisabled={working}
  onclose={close}
>
  <div class="glossary-editor">
    <div class="glossary-heading">
      <BookA size={24} />
      <div>
        <strong>{book.metadata.title}</strong><small
          >{tr(languageName(effective.sourceLanguage), $locale)} → {tr(
            languageName(effective.targetLanguage),
            $locale,
          )}</small
        >
      </div>
    </div>
    <div class="glossary-options">
      <ToggleSwitch
        field
        label={tr('Use glossary for this book', $locale)}
        title={tr('Use glossary for this book', $locale)}
        checked={draft.enabled}
        disabled={working}
        onchange={(value) => (draft.enabled = value)}
      />
      {#if service === 'llm'}<div class="glossary-auto-setting">
          <ToggleSwitch
            field
            title={tr('Automatically detect glossary terms', $locale)}
            label={tr('Automatically detect glossary terms', $locale)}
            checked={draft.autoDetect}
            disabled={!draft.enabled || working}
            onchange={(v) => (draft.autoDetect = v)}
          />
        </div>{/if}
    </div>
    {#if confirmClose}<div class="glossary-close-prompt" role="alert">
        <span>{tr('Discard glossary changes?', $locale)}</span><button
          disabled={working}
          onclick={() => {
            if (!working) confirmClose = false;
          }}>{tr('Keep editing', $locale)}</button
        ><button
          disabled={working}
          onclick={() => {
            if (!working) onclose();
          }}>{tr('Discard', $locale)}</button
        >
      </div>{/if}
    <fieldset class="glossary-fields" disabled={working}>
      {#if incoming && preview}
        <div class="glossary-import-heading">
          <h3>{tr('Review import', $locale)}</h3>
          <small>{importName}</small>
        </div>
        <div class="glossary-import-counts">
          <span>{tr('New terms', $locale)} <b>{preview.added}</b></span><span
            >{tr('Identical pairs', $locale)} <b>{preview.identical}</b></span
          ><span>{tr('Conflicts', $locale)} <b>{preview.conflicts}</b></span>
        </div>
        <label class="glossary-conflict-policy"
          >{tr('Conflicting translations', $locale)}<ComboBox
            label={tr('Conflicting translations', $locale)}
            bind:value={replace}
            options={[
              { value: false, label: 'Keep existing' },
              { value: true, label: 'Replace matching' },
            ]}
          /></label
        >
        <div class="glossary-table-scroll">
          <table class="glossary-table import-table">
            <thead
              ><tr
                ><th>{tr('Source term', $locale)}</th><th>{tr('Current translation', $locale)}</th
                ><th>{tr('Imported translation', $locale)}</th></tr
              ></thead
            ><tbody
              >{#each incoming.slice(importPage * 50, (importPage + 1) * 50) as term}<tr
                  ><td>{term.source}</td><td
                    >{draft.entries.find((e) => e.source.trim() === term.source)?.target || '—'}</td
                  ><td>{term.target}</td></tr
                >{/each}</tbody
            >
          </table>
        </div>
        <nav class="glossary-pagination" aria-label={tr('Import pages', $locale)}>
          <button
            disabled={importPage === 0}
            title={tr('Previous page', $locale)}
            aria-label={tr('Previous page', $locale)}
            onclick={() => importPage--}><ChevronLeft size={16} /></button
          ><span>{importPage + 1} / {importPages}</span><button
            disabled={importPage + 1 === importPages}
            title={tr('Next page', $locale)}
            aria-label={tr('Next page', $locale)}
            onclick={() => importPage++}><ChevronRight size={16} /></button
          >
        </nav>
      {:else}
        {#if service === 'deepl'}<div class="glossary-provider-note">
            <label
              >{tr('Hosted glossary ID (optional)', $locale)}<input
                bind:value={draft.deeplGlossaryId}
              /></label
            ><small
              >{tr(
                'DeepL uses this hosted glossary. Local entries are not uploaded or synchronized.',
                $locale,
              )}</small
            >
          </div>
        {:else if service !== 'llm'}<p class="glossary-provider-note">
            {tr(
              'Local glossary entries are used by LLM services. You can edit them now and use them later.',
              $locale,
            )}
          </p>{/if}
        <div class="glossary-tools">
          <label class="glossary-search"
            ><Search size={16} /><input
              aria-label={tr('Search terms', $locale)}
              placeholder={tr('Search terms', $locale)}
              value={search}
              oninput={(e) => {
                search = e.currentTarget.value;
                page = 0;
                selected = [];
              }}
            /></label
          ><button onclick={add}><Plus size={16} />{tr('Add term', $locale)}</button><button
            disabled={!selected.length}
            title={tr('Remove selected terms from this draft', $locale)}
            onclick={() => remove(selected)}
            ><Trash2 size={16} />{tr('Remove selected', $locale)}{#if selected.length}<span
                >{selected.length}</span
              >{/if}</button
          >
        </div>
        <div class="glossary-table-scroll">
          <table class="glossary-table">
            <thead
              ><tr
                ><th class="glossary-select"
                  ><input
                    type="checkbox"
                    aria-label={tr('Select terms on this page', $locale)}
                    checked={visible.length > 0 && visible.every((r) => selected.includes(r.id))}
                    disabled={!visible.length}
                    onchange={(e) =>
                      (selected = e.currentTarget.checked
                        ? [...new Set([...selected, ...visible.map((r) => r.id)])]
                        : selected.filter((id) => !visible.some((r) => r.id === id)))}
                  /></th
                ><th>{tr('Source term', $locale)}</th><th>{tr('Preferred translation', $locale)}</th
                ><th class="glossary-delete"></th></tr
              ></thead
            ><tbody>
              {#each visible as row (row.id)}<tr
                  ><td class="glossary-select"
                    ><input
                      type="checkbox"
                      aria-label={tr('Select term', $locale)}
                      checked={selected.includes(row.id)}
                      onchange={(e) =>
                        (selected = e.currentTarget.checked
                          ? [...selected, row.id]
                          : selected.filter((id) => id !== row.id))}
                    /></td
                  ><td
                    ><textarea
                      id={'glossary-source-' + row.id}
                      aria-label={tr('Source term', $locale)}
                      aria-invalid={!!errors[row.i]}
                      rows="2"
                      bind:value={row.e.source}
                    ></textarea>{#if draft.automaticSources.includes(row.e.source) && baseline.entries.some((e) => e.source === row.e.source && e.target === row.e.target)}<small
                        class="automatic-term">{tr('Automatic', $locale)}</small
                      >{/if}{#if errors[row.i]}<small class="glossary-row-error"
                        >{tr(errors[row.i], $locale)}</small
                      >{/if}</td
                  ><td
                    ><textarea
                      aria-label={tr('Preferred translation', $locale)}
                      aria-invalid={!!errors[row.i]}
                      rows="2"
                      bind:value={row.e.target}></textarea></td
                  ><td class="glossary-delete"
                    ><button
                      title={tr('Remove glossary entry', $locale)}
                      aria-label={tr('Remove glossary entry', $locale)}
                      onclick={() => remove([row.id])}><Trash2 size={16} /></button
                    ></td
                  ></tr
                >{/each}
            </tbody>
          </table>
          {#if !filtered.length}<div class="glossary-empty">
              <BookA size={32} />
              <p>{tr(search ? 'No matching terms' : 'No glossary terms yet', $locale)}</p>
              {#if !search}<button onclick={add}>{tr('Add term', $locale)}</button>{/if}
            </div>{/if}
        </div>
        <nav class="glossary-pagination" aria-label={tr('Glossary pages', $locale)}>
          <small>{filtered.length} / {draft.entries.length} {tr('terms', $locale)}</small><span
            class="spacer"
          ></span><button
            disabled={page === 0}
            title={tr('Previous page', $locale)}
            aria-label={tr('Previous page', $locale)}
            onclick={() => page--}><ChevronLeft size={16} /></button
          ><span>{page + 1} / {pages}</span><button
            disabled={page + 1 === pages}
            title={tr('Next page', $locale)}
            aria-label={tr('Next page', $locale)}
            onclick={() => page++}><ChevronRight size={16} /></button
          >
        </nav>
      {/if}
    </fieldset>
    <div class="glossary-status" role="status">
      {#if error}{tr(error, $locale)}{:else if refreshConflict}{tr(
          'Glossary changed elsewhere. Reopen it before saving; your draft is still available.',
          $locale,
        )}{:else if !valid && !incoming}{tr(
          'Fix invalid terms before saving or importing.',
          $locale,
        )}{:else if working}{tr('Working…', $locale)}{/if}
    </div>
  </div>
  {#snippet footer()}
    <div class="glossary-footer">
      {#if incoming}<button disabled={working} onclick={() => (incoming = null)}
          >{tr('Back', $locale)}</button
        ><span class="spacer"></span><button class="primary" disabled={working} onclick={merge}
          >{tr('Merge into draft', $locale)}</button
        >
      {:else}<input
          class="glossary-file"
          type="file"
          accept=".csv,.tsv"
          bind:this={input}
          onchange={(e) => void readFile(e.currentTarget.files?.[0])}
        />
        <div class="glossary-file-actions">
          <button
            disabled={working || !valid}
            title={tr('Import CSV or TSV terms into this draft', $locale)}
            onclick={() => input.click()}><Upload size={16} />{tr('Import', $locale)}</button
          ><ComboBox
            label={tr('Export format', $locale)}
            bind:value={exportFormat}
            disabled={working}
            options={[
              { value: 'csv', label: 'CSV' },
              { value: 'tsv', label: 'TSV' },
            ]}
          /><button
            disabled={working || !valid}
            title={tr('Export all draft terms without saving the book', $locale)}
            onclick={exportFile}><Download size={16} />{tr('Export', $locale)}</button
          >
        </div>
        <span class="spacer"></span>
        <div class="glossary-save-actions">
          <button disabled={working} onclick={close}>{tr('Cancel', $locale)}</button><button
            class="primary"
            disabled={working || !dirty || !valid}
            onclick={commit}>{tr('Save', $locale)}</button
          >
        </div>{/if}
    </div>
  {/snippet}
</Modal>

<style>
  .glossary-options {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding-bottom: 16px;
    border-bottom: 1px solid var(--border);
  }
  .glossary-auto-setting {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .automatic-term {
    display: inline-block;
    padding: 2px 7px;
    margin-top: 4px;
    background: var(--selection);
    color: var(--text);
    border-radius: 4px;
    font-size: 0.72em;
  }
  :global(.library-modal.wide.stretch:has(.glossary-editor)) {
    width: min(960px, calc(100vw - 32px));
    height: min(760px, calc(100dvh - 32px));
  }
  :global(.library-modal.wide.stretch:has(.glossary-editor) > .modal-body) {
    display: flex;
    padding: 0 24px;
    overflow: auto;
  }
  .glossary-editor {
    display: flex;
    flex-direction: column;
    gap: 16px;
    min-width: 0;
    min-height: 0;
    flex: 1;
  }
  .glossary-heading {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
  }
  .glossary-heading > div {
    flex: 1;
    min-width: 140px;
    overflow-wrap: anywhere;
  }
  .glossary-heading small {
    display: block;
    margin-top: 4px;
    color: var(--muted);
  }
  .glossary-fields {
    display: flex;
    flex-direction: column;
    gap: 14px;
    flex: 1;
    min-height: 120px;
    min-width: 0;
    padding: 0;
    border: 0;
    margin: 0;
  }
  .glossary-tools,
  .glossary-pagination,
  .glossary-footer,
  .glossary-file-actions,
  .glossary-save-actions,
  .glossary-close-prompt {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .glossary-search {
    flex-direction: row;
    margin: 0;
    display: flex;
    align-items: center;
    gap: 8px;
    flex: 1;
    min-width: 160px;
    padding: 0 10px;
    border: 1px solid var(--border);
    border-radius: 8px;
  }
  .glossary-search input {
    border: 0;
    min-width: 0;
    width: 100%;
    padding: 10px 0;
    background: transparent;
  }
  .glossary-tools button,
  .glossary-footer button {
    font-size: 0.9em;
  }
  .glossary-table-scroll {
    flex: 1;
    min-height: 120px;
    overflow: auto;
    scrollbar-gutter: stable;
    border: 1px solid var(--border);
    border-radius: 10px;
  }
  .glossary-table {
    width: 100%;
    border-collapse: collapse;
    table-layout: fixed;
  }
  .glossary-table th {
    position: sticky;
    top: 0;
    background: var(--raised);
    z-index: 1;
    text-align: left;
    font-size: 0.85em;
    color: var(--muted);
    padding: 10px;
  }
  .glossary-table td {
    padding: 10px;
    border-top: 1px solid var(--border);
    vertical-align: top;
    overflow-wrap: anywhere;
    white-space: pre-wrap;
  }
  .glossary-table textarea {
    width: 100%;
    min-width: 0;
    padding: 8px;
    resize: vertical;
    font: inherit;
    line-height: 1.5;
  }
  .glossary-table .glossary-select {
    width: 40px;
    text-align: center;
    padding: 12px 6px;
  }
  .glossary-table .glossary-delete {
    width: 44px;
    padding: 12px 4px;
  }
  .glossary-delete button {
    border: 0;
    background: transparent;
    padding: 6px;
    color: var(--muted);
  }
  .glossary-row-error {
    display: block;
    color: var(--error);
    margin-top: 5px;
  }
  .glossary-empty {
    display: flex;
    align-items: center;
    justify-content: center;
    flex-direction: column;
    color: var(--muted);
    min-height: 150px;
    gap: 12px;
    padding: 20px;
  }
  .glossary-empty p {
    margin: 0;
  }
  .glossary-pagination {
    font-size: 0.85em;
    color: var(--muted);
  }
  .glossary-pagination button {
    padding: 6px;
  }
  .glossary-status {
    min-height: 1.5em;
    font-size: 0.85em;
    color: var(--error);
    padding-bottom: 4px;
  }
  .glossary-footer {
    width: 100%;
  }
  .glossary-file-actions {
    flex-wrap: nowrap;
  }
  .glossary-file-actions :global(.combo) {
    width: 6.5em;
    min-width: 95px;
  }
  .glossary-file {
    display: none;
  }
  .glossary-provider-note {
    font-size: 0.85em;
    color: var(--muted);
    margin: 0;
  }
  .glossary-provider-note input {
    margin: 6px 0;
  }
  .glossary-import-heading h3 {
    margin: 0 0 4px;
  }
  .glossary-import-heading small {
    overflow-wrap: anywhere;
    color: var(--muted);
  }
  .glossary-import-counts {
    display: flex;
    gap: 20px;
    flex-wrap: wrap;
    font-size: 0.9em;
  }
  .glossary-import-counts b {
    margin-left: 6px;
  }
  .glossary-conflict-policy {
    max-width: 320px;
  }
  .glossary-close-prompt {
    border: 1px solid var(--border);
    padding: 12px;
    border-radius: 8px;
    background: var(--raised);
  }
  .glossary-close-prompt span {
    flex: 1;
  }
  @media (max-width: 640px) {
    :global(.library-modal.wide.stretch:has(.glossary-editor) > .modal-body) {
      padding: 0 12px;
    }
    .glossary-save-actions {
      margin-left: auto;
    }
    .glossary-table td {
      padding: 6px;
    }
  }
</style>
