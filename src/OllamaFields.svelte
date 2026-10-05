<script lang="ts">
  import { t, tr, locale, type UiText } from './i18n';
  import { ollamaDescription } from './model-descriptions';
  import ComboBox from './ComboBox.svelte';
  import { call } from './bridge';
  import { latestCheck, type CheckState } from './latest-check';
  import type { Provider, OllamaModels, OllamaStatus } from './types';
  let {
    provider = $bindable(),
    vision,
    onchange,
    onmetadata = () => {},
    onwarnings,
  }: {
    provider: Provider;
    vision: boolean;
    onchange: () => void;
    onmetadata?: () => void;
    onwarnings?: (messages: UiText[]) => void;
  } = $props();
  type Result = { endpoint: string; inventory: OllamaModels; detail: OllamaStatus };
  let revision = $state(0);
  let check = $state<CheckState<Result>>({
    value: null,
    fresh: false,
    pending: true,
    showActivity: false,
    error: '',
  });
  const loader = latestCheck<{ endpoint: string; model: string; force: boolean }, Result>(
    async (input) => {
      const [inventory, detail] = await Promise.all([
        call('ollama_models', { endpoint: input.endpoint, force: input.force }),
        call('ollama_model_details', input),
      ]);
      if (!inventory.connected) throw inventory.message || 'Cannot connect to Ollama.';
      return { endpoint: input.endpoint, inventory, detail };
    },
    (next) => {
      check = next;
      if (next.fresh) onmetadata();
    },
  );
  let force = false;
  $effect(() => {
    const endpoint = provider.endpoint,
      model = provider.model,
      request = revision;
    const refresh = force;
    force = false;
    return loader.start({ endpoint, model, force: refresh });
  });
  let models = $derived(
    check.value?.endpoint === provider.endpoint ? check.value.inventory.models : [],
  );
  let info = $derived(
    check.value?.endpoint === provider.endpoint && check.value.detail.model?.name === provider.model
      ? check.value.detail.model
      : null,
  );
  let message = $derived(
    check.error ||
      (check.value?.endpoint === provider.endpoint ? check.value?.detail.message : '') ||
      (info?.remote
        ? 'Choose a model installed on this server; cloud-backed models are unavailable here.'
        : info && !info.capabilities.includes('completion')
          ? 'Choose a model that supports text generation.'
          : vision && info && !info.capabilities.includes('vision')
            ? 'This model cannot read images. Choose a vision model or Local OCR.'
            : ''),
  );
  function refresh() {
    force = true;
    revision++;
    onchange();
  }
  $effect(() => {
    onwarnings?.(!check.showActivity && message ? [message] : []);
    return () => onwarnings?.([]);
  });
</script>

<label
  >{t('m_0a5a39673f27', $locale)}
  <div class="row">
    <input
      aria-label={t('m_0a5a39673f27', $locale)}
      bind:value={provider.endpoint}
      placeholder="http://localhost:11434"
      spellcheck="false"
    />
    <button title={t('m_25be3abf5dc0', $locale)} onclick={refresh}
      >{t('m_0e9161011702', $locale)}</button
    >
  </div>
</label>
<label
  >{t('m_5e2c614c23f0', $locale)}
  <ComboBox
    label={t('m_e09dcdf2cdfd', $locale)}
    editable
    bind:value={provider.model}
    placeholder={t('m_50c171ad25ef', $locale)}
    oncommit={onchange}
    options={models.map((m) => ({
      value: m.name,
      literal: true,
      label: m.name,
      description: ollamaDescription(m, info?.name === m.name ? info.capabilities : undefined),
      disabled:
        m.remote ||
        (!!info &&
          info.name === m.name &&
          (!info.capabilities.includes('completion') ||
            (vision && !info.capabilities.includes('vision')))),
    }))}
  />
</label>
<div class="ollama-status" aria-busy={check.pending}>
  <div class="choice-badges">
    <small>{t('m_c6e5660cecaa', $locale)}</small>
    <small
      >{check.error
        ? t('m_0303e1824670', $locale)
        : check.value?.endpoint === provider.endpoint
          ? t('m_22965568d22a', $locale)
          : t('m_7cb17add5d4c', $locale)}</small
    >
    {#if info}<small
        >{info.capabilities.includes('vision')
          ? t('m_b2a5e40eb844', $locale)
          : info.capabilities.includes('completion')
            ? t('m_8bbfc646f3f5', $locale)
            : t('m_acd45270f9b1', $locale)}</small
      ><small>{(info.size / 1024 ** 3).toFixed(1)} GB</small>{/if}
  </div>
  <div class="setup-status" role="status">
    {#if check.showActivity}{t('m_4d24731d814e', $locale)}{:else if message}<span class="error-text"
        >{tr(message, $locale)}</span
      >{:else if check.fresh && !models.length}{t('m_fbc3b34e23f3', $locale)}{/if}
  </div>
</div>
