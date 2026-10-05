<script lang="ts">
  import { uiError, type UiText, t, tr, locale } from './i18n';
  import { credentials } from './credentials';
  import { instructionIssue } from './instructions';
  import { warningKey, distinctWarnings, unseenWarnings } from './setup-warnings';
  import { credentialIdentity } from './credential-autosave';
  import { untrack } from 'svelte';
  import { cleanupName } from './model-descriptions';
  import PipelineFields from './PipelineFields.svelte';
  import ServiceFields from './ServiceFields.svelte';
  import LocalModelFields from './LocalModelFields.svelte';
  import { call, native } from './bridge';
  import {
    languageName,
    testSignature,
    readinessSignature,
    requiredModelIds,
    ollamaDestination,
  } from './setup-state';
  import { latestCheck, type CheckState } from './latest-check';
  import type { Settings, ModelPack, Readiness } from './types';
  let {
    settings = $bindable(),
    models,
    catalog,
    section = $bindable('pipeline'),
    navigation = true,
    bookMode = false,
    inspectionOutside = false,
    languages = true,
    sourceLocked = false,
    onerror,
    onclearerror,
    onreport,
    onlibrary,
    outsideWarnings = [],
  }: {
    settings: Settings;
    models: ModelPack[];
    catalog: Record<string, any>;
    section?: string;
    navigation?: boolean;
    bookMode?: boolean;
    inspectionOutside?: boolean;
    languages?: boolean;
    sourceLocked?: boolean;
    onerror: (e: unknown) => void;
    onclearerror: () => void;
    onreport?: (r: Readiness | null, fresh: boolean, visible: UiText[]) => void;
    onlibrary?: () => void;
    outsideWarnings?: UiText[];
  } = $props();
  let check = $state<CheckState<Readiness>>({
    value: null,
    fresh: false,
    pending: true,
    showActivity: false,
    error: '',
  });
  let promptIssue = $derived(
    instructionIssue(settings.providers.find((p) => p.id === settings.translation.providerId)),
  );
  let report = $derived.by(() => {
    if (!check.value) return null;
    const issues = check.value.issues.filter((issue) => issue.code !== 'instructions');
    if (promptIssue)
      issues.push({ code: 'instructions', section: 'services', message: promptIssue });
    return { ...check.value, issues, ready: !issues.length };
  });
  let requiredPacks = $derived(
    models
      .filter((m) => requiredModelIds(settings.translation, models).includes(m.id))
      .map((m) => ({
        id: m.id,
        name: m.name,
        status: report?.packs.find((p) => p.id === m.id)?.status || 'Checking…',
      })),
  );
  let serviceEpoch = $state(0);
  let epoch = $state(0),
    testBusy = $state(false),
    testResult = $state<UiText>('');
  let testKey = $state('');
  let provider = $derived(settings.providers.find((p) => p.id === settings.translation.providerId));
  const credentialStates = credentials.states;
  let credentialState = $derived(
    provider ? $credentialStates[credentialIdentity(provider)] : undefined,
  );
  let signature = $derived(testSignature(settings));
  let readinessKey = $derived(
    JSON.stringify([
      readinessSignature(settings),
      credentialState?.revision,
      credentialState?.pending,
    ]),
  );
  let checkedKey = $state('');
  let checkedEpoch = $state(-1);
  let fieldWarnings = $state<UiText[]>([]);
  let displayedFields = $derived(section === 'services' ? fieldWarnings : []);
  let fresh = $derived(
    !credentialState?.pending &&
      (!credentialState?.error || ['library', 'pipeline'].includes(section)) &&
      check.fresh &&
      checkedKey === readinessKey &&
      checkedEpoch === epoch,
  );
  const validation = latestCheck<{ settings: Settings; key: string; epoch: number }, Readiness>(
    async (input) => {
      const next = await call('translation_readiness', { settings: input.settings });
      return next;
    },
    (state) => {
      check = state;
    },
  );
  let visibleIssues = $derived(
    (report?.issues || [])
      .filter(
        (i) =>
          section === 'review' ||
          navigation ||
          i.section === section ||
          (section === 'models' && ['ocr', 'ocr_language'].includes(i.code)),
      )
      .filter(
        (issue) =>
          unseenWarnings([issue.message], [...displayedFields, ...outsideWarnings], $locale)
            .length > 0,
      )
      .filter(
        (issue, index, all) =>
          all.findIndex(
            (i) => warningKey(i.message, $locale) === warningKey(issue.message, $locale),
          ) === index,
      ),
  );
  function changed(service = false) {
    epoch++;
    if (service) {
      serviceEpoch++;
      testResult = '';
    }
    onclearerror();
  }
  $effect(() => {
    const key = readinessKey;
    const revision = epoch;
    checkedKey = key;
    checkedEpoch = revision;
    return validation.start({
      settings: untrack(() => $state.snapshot(settings)),
      key,
      epoch: revision,
    });
  });
  $effect(() => {
    onreport?.(
      report,
      fresh && !promptIssue,
      distinctWarnings(
        [
          ...displayedFields,
          ...outsideWarnings,
          ...visibleIssues.map((i) => i.message),
          check.error,
          ...(testResult && testKey === signature && !credentialState?.pending ? [testResult] : []),
        ],
        $locale,
      ),
    );
  });
  $effect(() => {
    if (credentialState?.pending) testResult = '';
  });
  async function test() {
    if (
      !fresh ||
      testBusy ||
      !report ||
      report.issues.some((i) => i.section === 'services' || ['languages', 'mode'].includes(i.code))
    )
      return;
    testBusy = true;
    testResult = '';
    const key = signature;
    const revision = serviceEpoch;
    const keyRevision = credentialState?.revision;
    try {
      const r = await call('service_test', {
        settings: $state.snapshot(settings),
      });
      if (
        key === signature &&
        revision === serviceEpoch &&
        keyRevision === credentialState?.revision
      ) {
        testKey = key;
        testResult = `${native ? 'Test succeeded' : 'Preview simulation succeeded (no request sent)'} · ${(r.elapsedMs / 1000).toFixed(1)}s · sample result: ${r.target}`;
      }
    } catch (e) {
      if (
        key === signature &&
        revision === serviceEpoch &&
        keyRevision === credentialState?.revision
      ) {
        testKey = key;
        testResult = uiError(e);
      }
    } finally {
      testBusy = false;
    }
  }
</script>

{#if navigation}<nav class="translation-tabs" aria-label={t('m_5810fd24fe86', $locale)}>
    <button class:active={section === 'pipeline'} onclick={() => (section = 'pipeline')}
      >{t('m_37e1c775f452', $locale)}</button
    ><button class:active={section === 'services'} onclick={() => (section = 'services')}
      >{t('m_604dce445e32', $locale)}</button
    ><button class:active={section === 'models'} onclick={() => (section = 'models')}
      >{t('m_8e4bf4368b03', $locale)}</button
    >
  </nav>{/if}
{#if section === 'pipeline'}<PipelineFields
    bind:settings
    {bookMode}
    {languages}
    {sourceLocked}
    onservice={() => (section = 'services')}
    serviceNavigation={navigation}
  />
{:else if section === 'services'}<ServiceFields
    bind:settings
    {catalog}
    {bookMode}
    {inspectionOutside}
    guided={!navigation}
    onchange={() => changed(true)}
    {onerror}
    onwarnings={(messages) => (fieldWarnings = messages)}
  />
{:else if section === 'models'}<LocalModelFields
    bind:settings
    {models}
    {report}
    {bookMode}
    guided={!navigation}
    onchange={changed}
    {onerror}
  />
{:else if section === 'review'}
  <dl class="setup-review">
    <dt>{t('m_318655cea4bd', $locale)}</dt>
    <dd>
      {tr(languageName(settings.translation.sourceLanguage), $locale)} → {tr(
        languageName(settings.translation.targetLanguage),
        $locale,
      )}
    </dd>
    <dt>{t('m_f2cba1900551', $locale)}</dt>
    <dd>
      {settings.translation.mode === 'vision'
        ? t('m_2cd1355967bd', $locale)
        : tr('Included detector →', $locale) +
          ' ' +
          (settings.translation.ocr === 'manga' ? 'Manga OCR' : 'PP-OCRv5') +
          ' → ' +
          tr(provider ? 'Text translation' : 'Manual translation', $locale)}
    </dd>
    <dt>{t('m_99d702d15c0e', $locale)}</dt>
    <dd>
      {provider
        ? `${provider.name} / ${provider.model || provider.service}`
        : settings.translation.mode === 'local'
          ? t('m_54880a560cfb', $locale)
          : t('m_df12aeba9bb9', $locale)}
    </dd>
    <dt>{t('m_66880d2d8216', $locale)}</dt>
    <dd>
      {provider?.protocol === 'ollama'
        ? t('m_c6e5660cecaa', $locale)
        : !report
          ? t('m_ec963ffc911b', $locale)
          : !report.credentialRequired
            ? t('m_c6e5660cecaa', $locale)
            : report.credentialStored
              ? t('m_535f102cff7a', $locale)
              : t('m_dd1841d29502', $locale)}
    </dd>
    {#if provider?.protocol === 'ollama'}<dt>Ollama</dt>
      <dd>
        {!report?.ollama
          ? t('m_7cb17add5d4c', $locale)
          : report.ollama.connected
            ? t('m_22965568d22a', $locale)
            : t('m_0303e1824670', $locale)}{#if report?.ollama?.model}
          · {report.ollama.model.capabilities.includes('vision')
            ? t('m_bf8592a9fa15', $locale)
            : t('m_3ba1d4be10bf', $locale)}{/if}
      </dd>{/if}
    <dt>{t('m_9f1b237b5dc3', $locale)}</dt>
    <dd>
      {tr(cleanupName(settings.translation.cleanup.method), $locale)} · {settings.translation
        .cleanup.method === 'solid'
        ? t('m_0b099867389c', $locale)
        : settings.translation.cleanup.strategy === 'automatic'
          ? t('m_b7aad4134de6', $locale)
          : t('m_0ed10a369cb6', $locale)}
    </dd>
    <dt>{t('m_d17d2d78d76e', $locale)}</dt>
    <dd>
      {#each requiredPacks as p (p.id)}<div>
          {p.name} · {p.status === 'verified'
            ? t('m_5fa7aac5375c', $locale)
            : tr(p.status, $locale)}
        </div>{/each}
    </dd>
    <dt>{t('m_dc20b3d5d2cd', $locale)}</dt>
    <dd>{settings.libraryDirectory}</dd>
  </dl>
{/if}
{#if section === 'services' && provider}
  <div class="setup-destination">
    <b
      >{settings.translation.mode === 'vision'
        ? t('m_247dedfb6552', $locale)
        : t('m_18fe6ac308de', $locale)}</b
    >
    <p>
      {provider.protocol === 'ollama'
        ? tr(ollamaDestination(provider.endpoint), $locale)
        : report?.destination || t('m_d1e363adcbec', $locale)}
    </p>
    <small
      >{provider.protocol === 'ollama'
        ? bookMode
          ? t('m_ceca0798943d', $locale)
          : ''
        : report?.credentialStored
          ? t('m_fde6af7fef11', $locale)
          : t('m_7ba22c59dfd6', $locale)}
      {testResult && testKey === signature && !credentialState?.pending
        ? ''
        : t('m_2044bd227930', $locale)}</small
    >
  </div>
{/if}
{#if provider && (section === 'review' || (navigation && section === 'services'))}
  {#if !bookMode}<div class="service-test">
      <button
        disabled={testBusy ||
          !fresh ||
          !report ||
          (report.credentialRequired && !report.credentialStored) ||
          report?.issues.some(
            (i) => i.section === 'services' || ['languages', 'mode'].includes(i.code),
          )}
        onclick={test}
        title={t('m_29f98fe90b55', $locale)}
        >{testBusy ? t('m_407b7a04662f', $locale) : t('m_3bd2a24a8bc7', $locale)}</button
      >{#if provider.protocol !== 'ollama'}<small>{t('m_b25293308777', $locale)}</small>{/if}
    </div>{/if}
  {#if testResult && testKey === signature && !credentialState?.pending}<p
      role="status"
      class="setup-help"
    >
      {tr(testResult, $locale)}
    </p>{/if}
{/if}
<div class="readiness-results" aria-busy={!fresh} class:pending={check.showActivity}>
  {#if visibleIssues.length}<div class="readiness-issues" aria-label={t('m_bc0de441a3e5', $locale)}>
      {#each visibleIssues as issue}<div>
          {#if navigation && ['pipeline', 'services', 'models'].includes(issue.section)}<button
              onclick={() => (section = issue.section)}
              title={t('m_6521e8b8a924', $locale)}>{tr(issue.message, $locale)}</button
            >{:else if navigation && issue.section === 'library' && onlibrary}<button
              onclick={onlibrary}
              title={t('m_e541b03e27e3', $locale)}>{tr(issue.message, $locale)}</button
            >{:else}<p>{tr(issue.message, $locale)}</p>{/if}
        </div>{/each}
    </div>{:else if report?.ready && navigation}<p class="setup-ready" role="status">
      {t('m_96f2f9c8af13', $locale)}
    </p>{/if}
</div>
<div class="setup-status" role="status" aria-live="polite">
  {#if check.error}<span class="error-text">{tr(check.error, $locale)}</span
    >{:else if check.showActivity}<small>{t('m_2b0ed71bd135', $locale)}</small>{/if}
</div>
