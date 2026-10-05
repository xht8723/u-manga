<script lang="ts">
  import {
    uiError,
    uiJoin,
    type UiText,
    t,
    tr,
    locale,
    setLanguage,
    type LanguagePreference,
  } from './i18n';
  import InterfaceLanguage from './InterfaceLanguage.svelte';
  import { credentials } from './credentials';
  import { unseenWarnings } from './setup-warnings';
  import { untrack } from 'svelte';
  import { Sun, Moon } from 'lucide-svelte';
  import Modal from './Modal.svelte';
  import LanguageFields from './LanguageFields.svelte';
  import TranslationSetup from './TranslationSetup.svelte';
  import { setupSteps, stepNames } from './setup-state';
  import { applyTheme } from './appearance';
  import { call, native } from './bridge';
  import { chooseLibrary } from './files';
  import type { Settings, ModelPack, Readiness } from './types';
  let {
    value,
    models,
    catalog,
    oncomplete,
  }: {
    value: Settings;
    models: ModelPack[];
    catalog: Record<string, any>;
    oncomplete: (s: Settings) => void;
  } = $props();
  let baseline = untrack(() => structuredClone($state.snapshot(value)));
  let draft = $state<Settings>(untrack(() => structuredClone($state.snapshot(value)))),
    step = $state(untrack(() => value.setup.step)),
    error = $state<UiText>(''),
    busy = $state(false),
    languageSaving = $state(false),
    fresh = $state(false),
    report = $state<Readiness | null>(null);
  let shownWarnings = $state<UiText[]>([]);
  let visibleWarnings = $derived(step === 'library' ? [] : shownWarnings);
  let steps = $derived(setupSteps(draft.translation.mode, draft.translation.cleanup.method)),
    index = $derived(steps.indexOf(step));
  let canAdvance = $derived(fresh && !!report?.advancement[step]?.ready);
  let blockedReason = $derived(
    uiJoin(report?.advancement[step]?.issues?.map((i) => i.message) || [], ' ') || '',
  );
  let distinctBlockedReason = $derived(
    uiJoin(
      unseenWarnings(
        report?.advancement[step]?.issues?.map((i) => i.message) || [],
        visibleWarnings,
        $locale,
      ),
      ' ',
    ),
  );
  let distinctError = $derived(unseenWarnings([error], visibleWarnings, $locale)[0]);
  async function changeLanguage(next: LanguagePreference) {
    if (busy || languageSaving) return;
    const previous = draft.uiLanguage;
    draft.uiLanguage = next;
    setLanguage(next);
    languageSaving = true;
    try {
      await call('onboarding_language', { language: next });
      baseline.uiLanguage = next;
    } catch (e) {
      draft.uiLanguage = previous;
      setLanguage(previous);
      error = uiError(e);
    } finally {
      languageSaving = false;
    }
  }
  async function advance(next: string, complete = false) {
    if (busy || languageSaving || ((complete || steps.indexOf(next) > index) && !canAdvance))
      return;
    busy = true;
    error = '';
    try {
      try {
        await credentials.flush(draft.providers.find((p) => p.id === draft.translation.providerId));
      } catch (e) {
        // Back remains available to repair the pipeline even if an immediate key save failed.
        if (complete || (steps.indexOf(next) > index && index >= steps.indexOf('services')))
          throw e;
      }
      draft = await call('onboarding_save', {
        settings: $state.snapshot(draft),
        base: baseline,
        step: next,
        complete,
      });
      baseline = structuredClone($state.snapshot(draft));
      step = next;
      if (complete) oncomplete(draft);
    } catch (e) {
      error = uiError(e);
    } finally {
      busy = false;
    }
  }
  async function switchTheme() {
    if (busy || languageSaving) return;
    const previous = structuredClone($state.snapshot(draft.appearance));
    languageSaving = true;
    draft.appearance.active = draft.appearance.active === 'day' ? 'night' : 'day';
    applyTheme(draft.appearance);
    try {
      await call('onboarding_appearance', { appearance: $state.snapshot(draft.appearance) });
      baseline.appearance = structuredClone($state.snapshot(draft.appearance));
    } catch (e) {
      draft.appearance = previous;
      applyTheme(previous);
      error = uiError(e);
    } finally {
      languageSaving = false;
    }
  }
</script>

<Modal
  title={t('m_994fcc38398a', $locale)}
  wide
  guided
  stretch
  scrollKey={step}
  closable={false}
  onclose={() => {}}
>
  {#snippet headerActions()}
    <button
      title={t('m_f7af7a4ddf17', $locale)}
      class="theme-switch"
      aria-label={t(
        draft.appearance.active === 'day' ? 'm_8f2364e11b8b' : 'm_4e9f8db8242b',
        $locale,
      )}
      disabled={busy || languageSaving}
      onclick={switchTheme}
      >{#if draft.appearance.active === 'day'}<Sun size={17} />{:else}<Moon
          size={17}
        />{/if}</button
    >
  {/snippet}
  <ol class="setup-steps" aria-label={t('m_1eba90226b10', $locale)}>
    {#each steps as s, i}<li
        class:current={step === s}
        class:done={i < index}
        aria-current={step === s ? 'step' : undefined}
      >
        <span>{i + 1}</span>{tr(stepNames[s], $locale)}
      </li>{/each}
  </ol>
  {#if !native}<small>{t('m_c57e223323aa', $locale)}</small>{/if}
  <h3 class="setup-title">
    <small>{index + 1} / {steps.length}</small>{tr(stepNames[step], $locale)}
  </h3>
  <fieldset disabled={busy || languageSaving} class="setup-fields">
    {#if step === 'library'}
      <InterfaceLanguage
        value={draft.uiLanguage}
        disabled={busy || languageSaving}
        onchange={changeLanguage}
      />
      <label
        >{t('m_955629b3151c', $locale)}
        <div class="row">
          <input
            readonly
            value={draft.libraryDirectory}
            placeholder={t('m_e87bf1f31a24', $locale)}
          /><button
            onclick={async () => {
              try {
                const path = await chooseLibrary();
                if (path) draft.libraryDirectory = path;
              } catch (e) {
                error = uiError(e);
              }
            }}>{t('m_3db741003e46', $locale)}</button
          >
        </div></label
      >
      <p class="setup-help">{t('m_1e952a4fb708', $locale)}</p>
      <LanguageFields bind:value={draft.translation} />
    {/if}
    <div hidden={step === 'library'}>
      <TranslationSetup
        bind:settings={draft}
        {models}
        {catalog}
        section={step}
        navigation={false}
        languages={false}
        onerror={(e) => (error = uiError(e))}
        onclearerror={() => (error = '')}
        onreport={(r, valid, shown) => {
          report = r;
          fresh = valid;
          shownWarnings = shown;
        }}
      />
    </div>
  </fieldset>
  <div class="setup-status" role="alert">
    {#if distinctError}<p class="error-text">
        {tr(distinctError, $locale)}
      </p>{:else if distinctBlockedReason}<p>
        {tr(distinctBlockedReason, $locale)}
      </p>{/if}
  </div>
  {#snippet footer()}<span class="spacer"></span>{#if index > 0}<button
        disabled={busy || languageSaving}
        onclick={() => advance(steps[index - 1])}>{t('m_76900f1bfd16', $locale)}</button
      >{/if}
    <button
      class="primary"
      disabled={busy || languageSaving || !canAdvance}
      title={tr(!fresh ? 'Checking requirements…' : blockedReason || undefined, $locale)}
      onclick={() => advance(step === 'review' ? 'review' : steps[index + 1], step === 'review')}
      >{busy
        ? t('m_23e39291d613', $locale)
        : step === 'review'
          ? t('m_bc01ae77f37c', $locale)
          : t('m_1ff57a29d7c9', $locale)}</button
    >
  {/snippet}
</Modal>
