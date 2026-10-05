<script lang="ts">
  import { t, tr, locale } from './i18n';
  import { BookOpen, Languages, Check } from 'lucide-svelte';
  import type { TranslationSettings } from './types';
  let { value = $bindable() }: { value: TranslationSettings } = $props();
</script>

<div class="method-cards ocr-choices" role="group" aria-label={t('m_c193761957ed', $locale)}>
  <button
    class:chosen={value.ocr === 'manga'}
    aria-pressed={value.ocr === 'manga'}
    disabled={value.sourceLanguage !== 'ja'}
    title={tr(value.sourceLanguage !== 'ja'
      ? 'Manga OCR supports Japanese only'
      : 'Read Japanese manga on your computer', $locale)}
    onclick={() => (value.ocr = 'manga')}
  >
    <span class="choice-icon"
      ><BookOpen size={28} />{#if value.ocr === 'manga'}<Check size={22} />{/if}</span
    >
    <strong>Manga OCR <span class="recommended">{t('m_d70604e84304', $locale)}</span></strong><span
      >{t('m_a4e127c4bd7a', $locale)}</span
    ><small>{t('m_ccf60b71a487', $locale)}</small>
  </button>
  <button
    class:chosen={value.ocr === 'pp'}
    aria-pressed={value.ocr === 'pp'}
    title={t('m_041d6e01fe16', $locale)}
    onclick={() => (value.ocr = 'pp')}
  >
    <span class="choice-icon"
      ><Languages size={28} />{#if value.ocr === 'pp'}<Check size={22} />{/if}</span
    >
    <strong>PP-OCRv5</strong><span>{t('m_dea2fbb92c76', $locale)}</span><small
      >{t('m_d0be1eef124d', $locale)}</small
    >
  </button>
</div>
