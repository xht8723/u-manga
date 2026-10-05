<script lang="ts">
  import { t, tr, locale, type UiText } from './i18n';
  import CredentialField from './CredentialField.svelte';
  import { credentialIdentity } from './credential-autosave';
  import { modelDescription } from './model-descriptions';
  import ComboBox from './ComboBox.svelte';
  import OllamaFields from './OllamaFields.svelte';
  import ThinkingFields from './ThinkingFields.svelte';
  import SimpleTranslationFields from './SimpleTranslationFields.svelte';
  import InstructionFields from './InstructionFields.svelte';
  import TranslationAdvanced from './TranslationAdvanced.svelte';
  import { onMount } from 'svelte';
  import { call } from './bridge';
  import { newProvider } from './setup-state';
  import type { Settings } from './types';
  let {
    settings = $bindable(),
    catalog,
    onchange,
    onerror,
    bookMode = false,
    guided = false,
    inspectionOutside = false,
    onwarnings,
  }: {
    settings: Settings;
    catalog: Record<string, any>;
    onchange: () => void;
    onerror: (e: unknown) => void;
    bookMode?: boolean;
    guided?: boolean;
    inspectionOutside?: boolean;
    onwarnings?: (messages: UiText[]) => void;
  } = $props();
  let provider = $derived(settings.providers.find((p) => p.id === settings.translation.providerId));
  let status = $state(''),
    chosenCatalog = $state(''),
    choice = $state(''),
    advancedOpen = $state(false);
  let generation = 0;
  let promptWarnings = $state<UiText[]>([]),
    ollamaWarnings = $state<UiText[]>([]);
  $effect(() => {
    onwarnings?.([
      ...(provider?.service === 'llm' && !inspectionOutside ? promptWarnings : []),
      ...(provider?.service === 'llm' && provider.protocol === 'ollama' && !bookMode
        ? ollamaWarnings
        : []),
    ]);
    return () => onwarnings?.([]);
  });
  let modelMetadataRevision = $state(0);
  // Recover the catalog selection from saved metadata without changing the saved
  // model or endpoint. Refreshes only replace the catalog, never the profile.
  $effect(() => {
    if (!provider && settings.translation.mode === 'local' && !choice) choice = 'none';
    if (settings.translation.mode === 'vision' && choice === 'none') choice = '';
    if (provider && !chosenCatalog) {
      const match = Object.entries(catalog).find(
        ([id, c]) =>
          c.name === provider!.name ||
          id === provider!.name ||
          (c.api && provider!.endpoint.replace(/\/$/, '') === c.api.replace(/\/$/, '')),
      );
      if (match) chosenCatalog = match[0];
      if (!choice)
        choice =
          provider.service !== 'llm'
            ? `service:${provider.service}`
            : provider.protocol === 'ollama'
              ? 'ollama'
              : match
                ? `catalog:${match[0]}`
                : 'custom';
    }
  });
  onMount(() => {
    if (!bookMode && provider?.protocol !== 'ollama') void refreshCatalog();
  });
  async function refreshCatalog() {
    const request = ++generation;
    try {
      const next = await call('catalog_refresh');
      if (request === generation) catalog = next;
    } catch {
      if (request === generation)
        status = 'Catalog unavailable. Cached choices and manual model IDs remain available.';
    }
  }
  function selectProvider() {
    status = '';
    chosenCatalog = '';
    choice = '';
    if (provider?.protocol !== 'ollama') void refreshCatalog();
    onchange();
  }
  function add() {
    const p = newProvider();
    settings.providers.push(p);
    settings.translation.providerId = p.id;
    selectProvider();
  }
  const services: Record<string, string> = {
    google: 'Google Cloud Translation',
    microsoft: 'Microsoft Translator',
    deepl: 'DeepL',
    baidu: 'Baidu Translate',
  };
  function configureChoice() {
    if (!choice) return;
    if (choice === 'none') {
      settings.translation.providerId = '';
      chosenCatalog = '';
      status = '';
      onchange();
      return;
    }
    if (!provider) {
      const draft = newProvider();
      settings.providers.push(draft);
      settings.translation.providerId = draft.id;
    }
    const p = settings.providers.find((p) => p.id === settings.translation.providerId)!;
    status = '';
    chosenCatalog = '';
    if (choice === 'ollama') {
      p.service = 'llm';
      p.protocol = 'ollama';
      p.name = 'Ollama';
      p.endpoint = 'http://localhost:11434';
      p.model = '';
      p.vision = false;
      advancedOpen = false;
    } else if (choice.startsWith('service:')) {
      p.service = choice.slice(8);
      p.protocol = 'openai';
      p.name = services[p.service];
      p.endpoint = '';
      p.model = '';
      p.vision = false;
    } else {
      p.service = 'llm';
      p.model = '';
      p.vision = false;
      chosenCatalog = choice.startsWith('catalog:') ? choice.slice(8) : '';
      const c = catalog[chosenCatalog];
      p.name = c?.name || chosenCatalog || 'Custom service';
      p.endpoint = c?.api || '';
      p.protocol = c?.npm?.includes('anthropic')
        ? 'anthropic'
        : c?.npm?.includes('google')
          ? 'gemini'
          : c?.npm === '@ai-sdk/openai'
            ? 'responses'
            : 'openai';
      advancedOpen = choice === 'custom';
    }
    onchange();
    if (choice !== 'ollama') void refreshCatalog();
  }
  function selectModel() {
    if (!provider) return;
    const m = catalog[chosenCatalog]?.models?.[provider.model];
    if (m) {
      provider.vision = m.modalities?.input?.includes('image') === true;
      if (!provider.endpoint.trim() && m.provider?.api) provider.endpoint = m.provider.api;
    }
  }
</script>

{#if !guided}<label
    >{t('m_8b2f02b773f8', $locale)}
    <div class="row">
      <ComboBox
        label={t('m_d054ef74d80d', $locale)}
        bind:value={settings.translation.providerId}
        oncommit={selectProvider}
        placeholder={t('m_55116f4b370e', $locale)}
        options={[
          ...(settings.translation.mode === 'local'
            ? [{ value: '', label: 'None · manual translation' }]
            : []),
          ...settings.providers.map((p) => ({
            value: p.id,
            literal: true,
            label: `${p.name} · ${p.model || p.service}`,
          })),
        ]}
      />
      {#if !bookMode}<button onclick={add} title={t('m_771db45488a3', $locale)}
          >{t('m_7d9366fdae36', $locale)}</button
        >{/if}
    </div></label
  >{/if}
{#if bookMode}<p class="setup-help">{t('m_4ac79c498682', $locale)}</p>
  {#if provider}<ThinkingFields
      bind:provider
      readonly
    />{#if !inspectionOutside}<SimpleTranslationFields bind:provider readonly />{/if}{/if}
{:else}
  {#if provider || guided}<label
      >{t('m_472590ae974d', $locale)}
      <div class="row">
        <ComboBox
          label={t('m_472590ae974d', $locale)}
          bind:value={choice}
          oncommit={configureChoice}
          placeholder={t('m_e1d36c3adeeb', $locale)}
          options={[
            ...(settings.translation.mode === 'local'
              ? [{ value: 'none', label: 'None · manual translation' }]
              : []),
            { value: 'ollama', label: 'Ollama · local or network server' },
            ...Object.entries(catalog).map(([id, c]) => ({
              value: `catalog:${id}`,
              literal: true,
              label: c.name || id,
            })),
            ...Object.entries(services).map(([id, label]) => ({
              value: `service:${id}`,
              label,
              disabled: settings.translation.mode === 'vision',
            })),
            { value: 'custom', label: 'Custom AI provider' },
          ]}
        />
        {#if provider?.protocol !== 'ollama'}<button
            title={t('m_735e9e0114ec', $locale)}
            onclick={refreshCatalog}>{t('m_0e9161011702', $locale)}</button
          >{/if}
      </div></label
    >{/if}
  {#if provider}
    {#if provider.protocol === 'ollama' && provider.service === 'llm'}
      <OllamaFields
        bind:provider
        vision={settings.translation.mode === 'vision'}
        {onchange}
        onmetadata={() => modelMetadataRevision++}
        onwarnings={(messages) => (ollamaWarnings = messages)}
      />
    {:else if provider.service === 'llm'}<label
        >{t('m_5e2c614c23f0', $locale)}<ComboBox
          label={t('m_089ef2b6dbb3', $locale)}
          editable
          bind:value={provider.model}
          oninput={() => {
            if (provider) provider.vision = false;
          }}
          oncommit={selectModel}
          placeholder={t('m_2e08d7e0db66', $locale)}
          options={Object.entries<any>(catalog[chosenCatalog]?.models || {}).map(([id, m]) => ({
            value: id,
            literal: true,
            label: id,
            description: modelDescription(m),
            disabled:
              settings.translation.mode === 'vision' && !m.modalities?.input?.includes('image'),
          }))}
        /></label
      >
      <small class="model-description"
        >{tr(modelDescription(catalog[chosenCatalog]?.models?.[provider.model]), $locale)}</small
      >
    {:else if provider.service === 'microsoft'}<label
        >{t('m_8210b07e4306', $locale)}<input bind:value={provider.region} /></label
      >
    {:else if provider.service === 'baidu'}<label
        >{t('m_77e90b89c040', $locale)}<input bind:value={provider.appId} /></label
      >{/if}
    <ThinkingFields bind:provider refreshKey={modelMetadataRevision} {onchange} />
    <SimpleTranslationFields bind:provider />
    {#if provider.protocol !== 'ollama' || provider.service !== 'llm'}
      {#key credentialIdentity(provider)}<CredentialField {provider} />{/key}
    {/if}
  {/if}
{/if}
<details class="setup-advanced service-advanced" bind:open={advancedOpen}>
  <summary>{tr('Advanced', $locale)}</summary>
  {#if provider && !bookMode}<div class="service-advanced-fields">
      <label>{t('m_2b7f6a84de91', $locale)}<input bind:value={provider.name} /></label>
      {#if provider.service === 'llm' && provider.protocol !== 'ollama'}
        <label
          >{t('m_cf0883343f4a', $locale)}<ComboBox
            label={t('m_cf0883343f4a', $locale)}
            bind:value={provider.protocol}
            options={[
              { value: 'openai', label: 'OpenAI-compatible Chat Completions' },
              { value: 'responses', label: 'OpenAI Responses' },
              { value: 'anthropic', label: 'Anthropic' },
              { value: 'gemini', label: 'Gemini' },
            ]}
          /></label
        >
        <label
          >{t('m_3df9726c68ba', $locale)}<input
            bind:value={provider.endpoint}
            placeholder={t('m_2cc23ff19474', $locale)}
          /></label
        >
        <label class="check"
          ><input type="checkbox" bind:checked={provider.vision} />{t(
            'm_43af65b56cf3',
            $locale,
          )}</label
        >
        <small>{t('m_9460b021c326', $locale)}</small>
      {/if}
      <label
        >{t('m_024729c31b8d', $locale)}<input
          type="number"
          min="0.1"
          max="20"
          step="0.1"
          bind:value={provider.rateLimit}
        /></label
      >
    </div>{/if}
  <TranslationAdvanced bind:settings {bookMode} />
</details>
{#if provider?.service === 'llm' && !inspectionOutside}<InstructionFields
    bind:provider
    settings={settings.translation}
    readonly={bookMode}
    onwarnings={(messages) => (promptWarnings = messages)}
  />{/if}
{#if status}<p class="setup-help" role="status">{tr(status, $locale)}</p>{/if}
