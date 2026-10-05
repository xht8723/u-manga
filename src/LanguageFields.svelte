<script lang="ts">
  import { t, tr, locale } from './i18n';
  import ComboBox from './ComboBox.svelte';
  import { languages, targetLanguages } from './setup-state';
  import type { TranslationSettings } from './types';
  let {
    value = $bindable(),
    sourceLocked = false,
  }: { value: TranslationSettings; sourceLocked?: boolean } = $props();
</script>

<div class="form-grid">
  <label
    >{t('m_088b23e3327d', $locale)}<ComboBox
      label={t('m_088b23e3327d', $locale)}
      bind:value={value.sourceLanguage}
      options={languages}
      disabled={sourceLocked}
    /></label
  >
  <label
    >{t('m_f83edd8c38bd', $locale)}<ComboBox
      label={t('m_f83edd8c38bd', $locale)}
      bind:value={value.targetLanguage}
      options={targetLanguages}
    /></label
  >
</div>
{#if sourceLocked}<small>{t('m_059141419597', $locale)}</small>{/if}
