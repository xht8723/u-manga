<script lang="ts">
  import { t, tr, uiError, locale } from './i18n';
  let { message }: { message: unknown } = $props();
</script>

<main class="startup-error">
  <section>
    <h1>{t('m_cd83c7d43793', $locale)}</h1>
    <p>{t('startup.help', $locale)}</p>
    <p class="startup-diagnostic" role="alert">{tr(uiError(message), $locale)}</p>
    <button class="primary" onclick={() => location.reload()}>{t('m_942087cc2d41', $locale)}</button>
  </section>
</main>

<style>
  .startup-error { min-height: 100dvh; display: grid; place-items: center; padding: max(24px, env(safe-area-inset-top)) 24px max(24px, env(safe-area-inset-bottom)); }
  section { width: min(100%, 560px); }
  h1 { font-size: 1.4rem; }
  p { line-height: 1.5; }
  .startup-diagnostic { overflow-wrap: anywhere; white-space: pre-wrap; color: var(--muted); margin: 24px 0; }
</style>
