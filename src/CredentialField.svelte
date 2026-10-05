<script lang="ts">
  import { t, tr, locale } from './i18n';
  import { onDestroy } from 'svelte';
  import { credentialIdentity } from './credential-autosave';
  import { credentials } from './credentials';
  import type { Provider } from './types';
  let { provider }: { provider: Provider } = $props();
  let value = $state('');
  const statuses = credentials.states;
  let status = $derived($statuses[credentialIdentity(provider)]);
  let snapshot = $derived($state.snapshot(provider));
  function flush() {
    void credentials.flush(snapshot).catch(() => {});
  }
  onDestroy(flush);
</script>

<label
  >{t('m_3737b659209e', $locale)}<input
    type="password"
    autocomplete="off"
    spellcheck="false"
    bind:value
    placeholder={t('m_4f633d08850d', $locale)}
    oninput={(event) => credentials.schedule(snapshot, event.currentTarget.value)}
    onblur={flush}
  /></label
>
<small>{t('m_cafec71eebab', $locale)}</small>
<div class="credential-status" role="status" aria-live="polite">
  {#if status?.error}<small class="error-text">{tr(status.error, $locale)}</small>
  {:else if status?.pending}<small>{t('m_9fb826a51ea8', $locale)}</small>
  {:else if status?.saved}<small>{t('m_9aa6f70741bc', $locale)}</small>{/if}
</div>
