<script lang="ts">
  import { t, tr, locale } from './i18n';
  let { translated, onchange }: { translated: boolean; onchange: (value: boolean) => void } =
    $props();
  function choose(value: boolean) {
    if (value !== translated) onchange(value);
  }
  function key(event: KeyboardEvent) {
    if (!['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key))
      return;
    event.preventDefault();
    event.stopPropagation();
    const value = event.key === 'Home' ? false : event.key === 'End' ? true : !translated;
    const group = (event.currentTarget as HTMLElement).parentElement!;
    group.querySelector<HTMLButtonElement>(`[data-translated="${value}"]`)?.focus();
    choose(value);
  }
</script>

<div class="page-view-selector" role="radiogroup" aria-label={t('m_d99d69879e52', $locale)}>
  {#each [false, true] as value}<button
      type="button"
      role="radio"
      data-translated={value}
      aria-checked={translated === value}
      tabindex={translated === value ? 0 : -1}
      title={tr(value ? 'Show the translated page' : 'Show the original page', $locale)}
      onkeydown={key}
      onclick={() => choose(value)}>{value ? t('m_abbc13c13e35', $locale) : t('m_88d759ea02ce', $locale)}</button
    >{/each}
</div>

<style>
  .page-view-selector {
    display: inline-flex;
    flex: none;
    padding: 3px;
    gap: 2px;
    border: 1px solid var(--border);
    border-radius: 9px;
    background: var(--window);
  }
  button {
    flex: 1;
    justify-content: center;
    min-width: 6.3em;
    min-height: 32px;
    padding: 6px 9px;
    border: 0;
    border-radius: 6px;
    color: var(--muted);
    background: transparent;
    font-size: 0.85em;
  }
  button[aria-checked='true'] {
    background: var(--selection);
    color: var(--text);
    box-shadow: inset 0 0 0 1px var(--accent);
    font-weight: 650;
  }
  button:focus-visible {
    outline: 2px solid var(--focus);
    outline-offset: 1px;
  }
</style>
