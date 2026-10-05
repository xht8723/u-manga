<script lang="ts">
  import { tr, locale } from './i18n';
  import ToggleSwitch from './ToggleSwitch.svelte';
  import type { Provider } from './types';
  let { provider = $bindable(), readonly = false }: { provider: Provider; readonly?: boolean } =
    $props();
</script>

{#if provider.service === 'llm'}
  <div class="simple-translation-field">
    <ToggleSwitch
      label={tr('Simple translation mode', $locale)}
      title={tr(
        readonly ? 'Managed by the selected service profile.' : 'Simple translation mode',
        $locale,
      )}
      checked={provider.simpleTranslation}
      disabled={readonly}
      focusableWhenDisabled
      onchange={(value) => {
        provider.simpleTranslation = value;
      }}
    />
    <small
      >{tr(
        readonly
          ? 'Managed by the selected service profile.'
          : 'Translate a page’s text together using numbered replies. Large pages are split.',
        $locale,
      )}</small
    >
  </div>
{/if}

<style>
  .simple-translation-field {
    margin: 12px 0 18px;
  }
  small {
    display: block;
    color: var(--muted);
    margin: 5px 10px;
    line-height: 1.5;
  }
</style>
