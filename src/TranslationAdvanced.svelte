<script lang="ts">
  import { tr, locale } from './i18n';
  import type { Settings } from './types';
  let { settings = $bindable(), bookMode = false }: { settings: Settings; bookMode?: boolean } =
    $props();
  let provider = $derived(settings.providers.find((p) => p.id === settings.translation.providerId));
  let supportsContext = $derived(provider?.service === 'llm' || provider?.service === 'deepl');
</script>

<div class="translation-advanced">
  <div class="advanced-grid">
    <label
      >{tr('Previous context pages', $locale)}
      <input
        aria-label={tr('Previous context pages', $locale)}
        type="number"
        min="0"
        max="20"
        bind:value={settings.translation.contextPages}
        disabled={!supportsContext}
      />
      <small
        >{tr(
          supportsContext
            ? 'Uses available original dialogue from preceding pages. Zero disables context.'
            : 'This service does not support preceding-page context.',
          $locale,
        )}</small
      >
    </label>
    {#if !bookMode}
      <label
        >{tr('Concurrent books', $locale)}
        <input
          aria-label={tr('Concurrent books', $locale)}
          type="number"
          min="1"
          max="4"
          bind:value={settings.concurrentBooks}
        />
        <small>{tr('Each book processes one page at a time.', $locale)}</small>
      </label>
    {/if}
  </div>
</div>

<style>
  .advanced-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(100%, 220px), 1fr));
    gap: 18px 24px;
    margin-top: 16px;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  small {
    color: var(--muted);
    font-weight: normal;
    line-height: 1.5;
  }
</style>
