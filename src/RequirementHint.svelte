<script lang="ts">
  import { type UiText, t, tr, locale } from './i18n';
  import { hosted } from './runtime-mode';
  let {
    reason,
    onsettings,
    disabled = false,
  }: { reason: UiText; onsettings: () => void; disabled?: boolean } = $props();
</script>

{#if reason}<div class="requirement-hint" role="note" title={tr(reason, $locale)}>
    <span>{tr(reason, $locale)}{#if hosted} {t('host.configurePC', $locale)}{/if}</span>{#if !hosted && reason !== 'Checking requirements…'}<button
        {disabled}
        onclick={onsettings}
        title={tr(reason, $locale)}
        aria-label={tr(
          `${tr(reason, $locale)} ${tr('Open Translation settings.', $locale)}`,
          $locale,
        )}>{t('m_74a883a037bc', $locale)}</button
      >{/if}
  </div>{/if}

<style>
  .requirement-hint {
    display: flex;
    gap: 8px;
    align-items: center;
    font-size: 0.78em;
    color: var(--muted);
    padding: 5px 0;
    overflow-wrap: anywhere;
  }
  .requirement-hint span {
    flex: 1;
    min-width: 0;
  }
  .requirement-hint button {
    font-size: inherit;
    flex: none;
    padding: 4px 7px;
    min-height: 26px;
  }
</style>
