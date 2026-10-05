<script lang="ts">
  import { validateInstructions } from './instructions';
  import { unseenWarnings } from './setup-warnings';
  import InstructionFields from './InstructionFields.svelte';
  import SimpleTranslationFields from './SimpleTranslationFields.svelte';
  import {
    uiError,
    type UiText,
    t,
    tr,
    locale,
    setLanguage,
    type LanguagePreference,
  } from './i18n';
  import InterfaceLanguage from './InterfaceLanguage.svelte';
  import { credentials } from './credentials';
  import ComboBox from './ComboBox.svelte';
  import { untrack } from 'svelte';
  import { open } from '@tauri-apps/plugin-dialog';
  import Modal from './Modal.svelte';
  import AppearanceFields from './AppearanceFields.svelte';
  import TranslationSetup from './TranslationSetup.svelte';
  import { effectiveSettings } from './book-state';
  import { applyTheme } from './appearance';
  import { call, native } from './bridge';
  import { chooseLibrary } from './files';
  import { modelDirectory } from './model-storage';
  import type { Settings, ModelPack, Book } from './types';
  let {
    value,
    models,
    catalog,
    fonts,
    onclose,
    onapply,
    onrelocate,
    book,
    onbook,
    onpreview,
    initialTab = 'General',
  }: {
    value: Settings;
    models: ModelPack[];
    catalog: Record<string, any>;
    fonts: string[];
    onclose: () => void;
    onapply: (s: Settings, base?: Settings) => Promise<void>;
    onrelocate: (destination: string) => Promise<Settings>;
    book?: Book;
    onbook?: (b: Book) => void;
    onpreview?: (a: Settings['appearance']) => void;
    initialTab?: string;
  } = $props();
  const baseline = untrack(() => structuredClone($state.snapshot(value)));
  let draft = $state<Settings>(untrack(() => structuredClone($state.snapshot(value)))),
    tab = $state(untrack(() => (book ? 'Translation' : initialTab))),
    error = $state<UiText>(''),
    working = $state(false);
  let shownWarnings = $state<UiText[]>([]),
    outsideWarnings = $state<UiText[]>([]);
  let distinctError = $derived(
    unseenWarnings([error], tab === 'Translation' ? shownWarnings : [], $locale)[0],
  );
  let overrideTranslation = $state(untrack(() => !!book?.overrides.translation)),
    overrideReader = $state(untrack(() => !!book?.overrides.reader));
  untrack(() => {
    if (book) {
      draft.translation = effectiveSettings($state.snapshot(book), $state.snapshot(value));
      draft.reader = structuredClone($state.snapshot(book.overrides.reader || value.reader));
      if (book.metadata.language) draft.translation.sourceLanguage = book.metadata.language;
    }
  });
  const tabs = $derived(
    book ? ['Reader', 'Translation'] : ['General', 'Appearance', 'Translation', 'Storage/About'],
  );
  const pendingPreferences = $derived(JSON.stringify(draft) !== JSON.stringify(baseline));
  const instructionProvider = $derived(
    draft.providers.find((p) => p.id === draft.translation.providerId),
  );
  async function directory() {
    const path = await chooseLibrary();
    if (path) draft.libraryDirectory = path;
  }
  function cancel() {
    if (working) return;
    applyTheme(value.appearance);
    if (!book) setLanguage(value.uiLanguage);
    onclose();
  }
  async function apply() {
    if (working) return;
    const submitted = structuredClone($state.snapshot(draft));
    const target = book ? structuredClone($state.snapshot(book)) : null;
    const translationOverride = overrideTranslation;
    const readerOverride = overrideReader;
    working = true;
    error = '';
    try {
      validateInstructions(submitted.providers);
      await credentials.flush(
        submitted.providers.find((p) => p.id === submitted.translation.providerId),
      );
      if (target) {
        const updated = await call('book_update', {
          path: target.path,
          metadata: target.metadata,
          expected: target.revision,
          overrides: {
            translation: translationOverride ? submitted.translation : null,
            reader: readerOverride ? submitted.reader : null,
          },
        });
        onbook?.(updated);
        onclose();
      } else {
        await onapply(submitted, baseline);
        onclose();
      }
    } catch (e) {
      error = uiError(e);
    } finally {
      working = false;
    }
  }
</script>

<Modal
  title={tr(book ? 'Book settings' : 'Settings', $locale)}
  wide
  closable={!working}
  onclose={cancel}
>
  <div class="settings-layout" inert={working}>
    <nav class="settings-nav" aria-label={t('m_e26d51d36781', $locale)}>
      {#each tabs as name}<button class:active={tab === name} onclick={() => (tab = name)}
          >{tr(name, $locale)}</button
        >{/each}
    </nav>
    <section class="settings-content">
      <h3>{tr(tab, $locale)}</h3>
      {#if tab === 'General'}<InterfaceLanguage
          value={draft.uiLanguage}
          disabled={working}
          onchange={(next) => {
            draft.uiLanguage = next;
            setLanguage(next);
          }}
        />
        <section class="general-group">
          <h4>{tr('Library', $locale)}</h4>
          <label
            >{t('m_955629b3151c', $locale)}
            <div class="row library-folder-controls">
              <input readonly value={draft.libraryDirectory} /><button
                title={t('m_a0955f81912f', $locale)}
                onclick={() => directory()}>{t('m_c7f937836f5d', $locale)}</button
              >
              <button
                class="move-library"
                title={pendingPreferences
                  ? tr('Apply or cancel changes first.', $locale)
                  : t('m_0f263b85d722', $locale)}
                disabled={!native || working || !value.libraryDirectory}
                aria-disabled={pendingPreferences || !native || working || !value.libraryDirectory}
                aria-describedby={pendingPreferences ? 'move-library-pending' : undefined}
                onclick={async () => {
                  if (pendingPreferences || working || !native || !value.libraryDirectory) return;
                  const beforeRoot = value.libraryDirectory;
                  working = true;
                  try {
                    const destination = await open({
                      directory: true,
                      title: tr('Move library to empty folder'),
                    });
                    if (typeof destination === 'string') {
                      working = true;
                      await onrelocate(destination);
                      onclose();
                    }
                  } catch (e) {
                    // The shell also publishes committed activations with recovery
                    // warnings; retain that acknowledgement without another preflight.
                    if (value.libraryDirectory !== beforeRoot)
                      draft = structuredClone($state.snapshot(value));
                    error = uiError(e);
                  } finally {
                    working = false;
                  }
                }}>{t('m_82d18ef9e480', $locale)}</button
              >
            </div></label
          >
          {#if pendingPreferences}<span hidden id="move-library-pending"
              >{tr('Apply or cancel changes first.', $locale)}</span
            >{/if}
          <label
            >{t('m_686057af598e', $locale)}<ComboBox
              label={tr('Default sorting', $locale)}
              bind:value={draft.librarySort}
              options={[
                { value: 'title', label: 'Title', disabled: false },
                { value: 'creator', label: 'Creator', disabled: false },
                { value: 'updated', label: 'Recently changed', disabled: false },
              ]}
            /></label
          >
        </section>{/if}
      {#if tab === 'General' || tab === 'Reader'}<section
          class="general-group"
          class:reader-defaults={!book}
        >
          {#if !book}<h4>{tr('Reader', $locale)}</h4>{/if}{#if book}<label
              class="check override-row"
              ><input type="checkbox" bind:checked={overrideReader} />{t(
                'm_fa3f0459d416',
                $locale,
              )}</label
            >{/if}
          <fieldset disabled={!!book && !overrideReader}>
            <label
              >{t('m_a511909161e1', $locale)}<ComboBox
                label={tr('Layout', $locale)}
                bind:value={draft.reader.layout}
                options={[
                  { value: 'paged', label: 'Paged', disabled: false },
                  { value: 'continuous', label: 'Continuous', disabled: false },
                ]}
              /></label
            >{#if !book}<label
                >{t('m_fbd284f4229f', $locale)}<input
                  type="range"
                  min="25"
                  max="160"
                  step="5"
                  bind:value={draft.reader.zoom}
                /><output>{draft.reader.zoom}%</output></label
              >{/if}
          </fieldset>
        </section>
      {:else if tab === 'Appearance'}<AppearanceFields
          value={draft.appearance}
          {fonts}
          onpreview={(a) => {
            draft.appearance = a;
            applyTheme(a);
            onpreview?.(a);
          }}
          onapply={() => {}}
          oncancel={() => {}}
        />
      {:else if tab === 'Translation'}{#if book}<label class="check override-row"
            ><input type="checkbox" bind:checked={overrideTranslation} />{t(
              'm_2804f797bf96',
              $locale,
            )}</label
          >{/if}
        <fieldset disabled={!!book && !overrideTranslation}>
          <TranslationSetup
            bind:settings={draft}
            onlibrary={book ? undefined : () => (tab = 'General')}
            {models}
            {catalog}
            sourceLocked={!!book?.metadata.language}
            bookMode={!!book}
            inspectionOutside={!!book && !overrideTranslation}
            onerror={(e) => (error = uiError(e))}
            onclearerror={() => (error = '')}
            {outsideWarnings}
            onreport={(_report, _fresh, shown) => (shownWarnings = shown)}
          />
        </fieldset>
        {#if book && !overrideTranslation && instructionProvider?.service === 'llm'}
          <SimpleTranslationFields provider={instructionProvider} readonly />
          <InstructionFields
            provider={instructionProvider}
            settings={draft.translation}
            readonly
            onwarnings={(messages) => (outsideWarnings = messages)}
          />
        {/if}
      {:else}<dl>
          <dt>{t('m_dd167905de0d', $locale)}</dt>
          <dd>U-Manga 0.1.0</dd>
          <dt>{t('m_dc20b3d5d2cd', $locale)}</dt>
          <dd>{value.libraryDirectory || t('m_df12aeba9bb9', $locale)}</dd>
          <dt>{t('m_9739ccce227e', $locale)}</dt>
          <dd>{modelDirectory(value) || t('m_c2f98126ed2d', $locale)}</dd>
          <dt>{t('m_bffd11bf9a0f', $locale)}</dt>
          <dd>{t('m_3a8ffd62d996', $locale)}</dd>
        </dl>
        <div class="row">
          <button
            title={t('m_8ef391c06ebd', $locale)}
            disabled={!value.libraryDirectory}
            onclick={async () => {
              try {
                await call('show_source', { path: value.libraryDirectory });
              } catch (e) {
                error = uiError(e);
              }
            }}>{t('m_c43b69c66a06', $locale)}</button
          ><button
            disabled={working}
            title={t('m_ede46a033e74', $locale)}
            onclick={async () => {
              try {
                working = true;
                await call('library_clear_thumbnails');
                error = 'Generated thumbnails cleared. Uploaded covers and edits are preserved.';
              } catch (e) {
                error = uiError(e);
              } finally {
                working = false;
              }
            }}>{t('m_5546b13e0ef8', $locale)}</button
          ><button
            title={t('m_f67b70a8bdea', $locale)}
            onclick={async () => {
              try {
                await call('show_notices');
              } catch (e) {
                error = uiError(e);
              }
            }}>{t('m_8fadbc8de273', $locale)}</button
          >
        </div>{/if}
      {#if distinctError}<p class="error-text" role="alert">{tr(distinctError, $locale)}</p>{/if}
    </section>
  </div>
  {#snippet footer()}<span class="spacer"></span><button disabled={working} onclick={cancel}
      >{t('m_19766ed6ccb2', $locale)}</button
    ><button class="primary" disabled={working} onclick={apply}
      >{t('m_31e392d1c037', $locale)}</button
    >{/snippet}
</Modal>

<style>
  .library-folder-controls {
    flex-wrap: wrap;
    gap: 8px;
  }
  .library-folder-controls input {
    flex: 1 1 240px;
    min-width: 0;
  }
  .library-folder-controls button {
    flex: none;
  }
  .move-library[aria-disabled='true'] {
    opacity: 0.55;
    cursor: default;
  }
  .general-group {
    display: grid;
    gap: 12px;
  }
  .general-group h4 {
    margin: 0 0 4px;
    font-size: 1em;
  }
  .general-group :global(label),
  .general-group :global(small) {
    margin-bottom: 0;
  }
  .general-group fieldset {
    display: grid;
    gap: 16px;
  }
  .reader-defaults {
    margin-top: 28px;
    padding-top: 22px;
    border-top: 1px solid var(--border);
  }
</style>
