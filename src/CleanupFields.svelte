<script lang="ts">
  import { t, tr, locale } from './i18n';
  import { Check, Paintbrush, Sparkles } from 'lucide-svelte';
  import ComboBox from './ComboBox.svelte';
  import type { CleanupSettings } from './types';
  import { cleanupChoices } from './model-descriptions';
  let { value = $bindable() }: { value: CleanupSettings } = $props();
</script>

<div class="method-cards cleanup-choices" role="group" aria-label={t('m_315ed2e1b8a8', $locale)}>
  {#each cleanupChoices as choice}
    <button
      class:chosen={value.method === choice.value}
      aria-pressed={value.method === choice.value}
      title={tr(choice.description, $locale)}
      onclick={() => (value.method = choice.value)}
    >
      <span class="choice-icon"
        >{#if choice.value === 'solid'}<Paintbrush size={28} />{:else}<Sparkles
            size={28}
          />{/if}{#if value.method === choice.value}<Check size={22} />{/if}</span
      >
      <strong
        >{tr(choice.label, $locale)}
        {#if choice.value === 'manga_lama'}<span class="recommended"
            >{t('m_d70604e84304', $locale)}</span
          >{/if}</strong
      ><span>{tr(choice.description, $locale)}</span>
      <span class="choice-badges"
        ><small>{tr(choice.size, $locale)}</small>{#if choice.value !== 'solid'}<small
            >{t('m_eac40e847ed5', $locale)}</small
          >{/if}</span
      >
    </button>
  {/each}
</div>
{#if value.method !== 'solid'}
  <div class="cleanup-options">
    <label class="pipeline-device">
      <span>
        <strong>{t('m_e182eaf509a5', $locale)}</strong>
      </span>
      <ComboBox
        label={t('m_e182eaf509a5', $locale)}
        bind:value={value.device}
        options={[
          {
            value: 'directml',
            label: 'GPU (DirectML)',
            description: 'Use the GPU for inpainting; report any CPU fallback.',
          },
          { value: 'cpu', label: 'CPU', description: 'Inpaint using four CPU threads only.' },
          {
            value: 'auto',
            label: 'Automatic (GPU preferred)',
            description: 'Try DirectML when available, then CPU if needed.',
          },
        ]}
      />
    </label>

    <label
      >{t('m_c8e3e92a62ec', $locale)}<ComboBox
        label={t('m_e698146617d2', $locale)}
        bind:value={value.strategy}
        options={[
          {
            value: 'automatic',
            label: 'Automatic: fill + inpainting',
            description: 'Fill plain interiors and preserve full context for inpainting.',
          },
          {
            value: 'all_regions',
            label: 'Inpaint every text region',
            description: 'Use the model throughout; takes more processing time.',
          },
        ]}
      /></label
    >
  </div>
{/if}
