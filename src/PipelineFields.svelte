<script lang="ts">
  import { t, tr, locale } from './i18n';
  import { Eye, ScanText, Check } from 'lucide-svelte';
  import OcrChoices from './OcrChoices.svelte';
  import CleanupFields from './CleanupFields.svelte';
  import ComboBox from './ComboBox.svelte';
  import LanguageFields from './LanguageFields.svelte';
  import type { Settings } from './types';
  let {
    settings = $bindable(),
    bookMode = false,
    languages = true,
    sourceLocked = false,

    onservice,
    serviceNavigation = true,
  }: {
    settings: Settings;
    bookMode?: boolean;
    languages?: boolean;
    sourceLocked?: boolean;

    onservice?: () => void;
    serviceNavigation?: boolean;
  } = $props();
  let s = $derived(settings.translation);
  let provider = $derived(settings.providers.find((p) => p.id === s.providerId));
</script>

{#if languages}<LanguageFields bind:value={settings.translation} {sourceLocked} />{/if}
<div class="pipeline-sections">
  <section class="pipeline-stage detection-stage" aria-label={t('m_343277142af2', $locale)}>
    <header>
      <span class="stage-number">1</span>
      <h4>{t('m_343277142af2', $locale)}</h4>
      <span class="tag"><Check size={14} />{t('m_ba829a98b799', $locale)}</span>
    </header>
    <p><strong>RT-DETR</strong> {t('m_a13ced272df9', $locale)}</p>
  </section>
  <section class="pipeline-stage" aria-label={t('m_2f482e90893b', $locale)}>
    <header>
      <span class="stage-number">2</span>
      <h4>{t('m_2f482e90893b', $locale)}</h4>
    </header>
    <div class="method-cards" role="group" aria-label={t('m_e63fcc61d0a9', $locale)}>
      <button
        class:chosen={s.mode === 'vision'}
        aria-pressed={s.mode === 'vision'}
        onclick={() => (s.mode = 'vision')}
      >
        <span class="choice-icon"
          ><Eye size={30} />{#if s.mode === 'vision'}<Check size={22} />{/if}</span
        >
        <strong>{t('m_c587c2601ccf', $locale)}</strong><span>{t('m_ef52a52a240d', $locale)}</span>
        <span class="choice-badges"
          ><small>{t('m_c27608a344af', $locale)}</small><small>{t('m_18b33d05aedb', $locale)}</small
          ></span
        >
      </button>
      <button
        class:chosen={s.mode === 'local'}
        aria-pressed={s.mode === 'local'}
        onclick={() => (s.mode = 'local')}
      >
        <span class="choice-icon"
          ><ScanText size={30} />{#if s.mode === 'local'}<Check size={22} />{/if}</span
        >
        <strong
          >{t('m_cde512840629', $locale)}
          <span class="recommended">{t('m_d70604e84304', $locale)}</span></strong
        ><span>{t('m_ce8a02285540', $locale)}</span>
        <span class="choice-badges"
          ><small>{t('m_7e6994a7874d', $locale)}</small><small>{t('m_846c88156d7b', $locale)}</small
          ></span
        >
      </button>
    </div>
    {#if s.mode === 'local'}<div class="pipeline-engine">
        <h5>{t('m_9beeb843d2e5', $locale)}</h5>
        <OcrChoices bind:value={settings.translation} />
        <label class="pipeline-device">
          <span>
            <strong>{t('m_51be1defdd17', $locale)}</strong>
          </span>
          <ComboBox
            label={t('m_51be1defdd17', $locale)}
            bind:value={s.device}
            options={[
              {
                value: 'directml',
                label: 'GPU (DirectML)',
                description: 'Checks recognition against CPU before using the GPU.',
              },
              { value: 'cpu', label: 'CPU', description: 'Read text using the CPU only.' },
            ]}
          />
        </label>
      </div>{/if}
  </section>
  <section class="pipeline-stage" aria-label={t('m_bbd8aad83c24', $locale)}>
    <header>
      <span class="stage-number">3</span>
      <h4>{t('m_bbd8aad83c24', $locale)}</h4>
    </header>
    <div class="pipeline-service">
      <div>
        <strong
          >{provider?.name ||
            (s.mode === 'local'
              ? t('m_54880a560cfb', $locale)
              : t('m_e327d425e613', $locale))}</strong
        >
        <p>
          {provider
            ? provider.model || provider.service
            : s.mode === 'local'
              ? t('m_e27e5c6f8928', $locale)
              : serviceNavigation
                ? t('m_1684f0a8017d', $locale)
                : t('m_65bb33ba5088', $locale)}
        </p>
      </div>
      {#if serviceNavigation}<button title={t('m_70606e100a58', $locale)} onclick={onservice}
          >{t('m_4ae3f62e0a4e', $locale)}</button
        >{/if}
    </div>
  </section>
  <section class="pipeline-stage" aria-label={t('m_2a77d3ec60b4', $locale)}>
    <header>
      <span class="stage-number">4</span>
      <h4>{t('m_2a77d3ec60b4', $locale)}</h4>
    </header>
    <CleanupFields bind:value={settings.translation.cleanup} />
  </section>
</div>
