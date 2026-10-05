<script lang="ts">
  import { t, tr, locale } from './i18n';
  let {
    checked,
    label,
    title,
    wide = false,
    field = false,
    disabled = false,
    focusableWhenDisabled = false,
    onchange,
  }: {
    checked: boolean;
    label: string;
    title: string;
    wide?: boolean;
    field?: boolean;
    disabled?: boolean;
    focusableWhenDisabled?: boolean;
    onchange: (checked: boolean) => void;
  } = $props();
</script>

<button
  type="button"
  class="toggle-switch"
  class:wide
  class:field
  role="switch"
  aria-label={tr(label, $locale)}
  aria-checked={checked}
  title={tr(title, $locale)}
  disabled={disabled && !focusableWhenDisabled}
  aria-disabled={disabled}
  onclick={() => {
    if (!disabled) onchange(!checked);
  }}
>
  <span class="toggle-label">{tr(label, $locale)}</span>
  <span class="toggle-track" aria-hidden="true"><span></span></span>
</button>

<style>
  .toggle-switch {
    display: inline-flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 7px 10px;
    min-height: 34px;
    border: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    text-align: left;
  }
  .toggle-switch:hover {
    background: var(--raised);
  }
  .toggle-switch[aria-disabled='true'] {
    opacity: 0.55;
    cursor: default;
  }
  .wide {
    width: 100%;
  }
  .field {
    align-self: flex-start;
    width: fit-content;
    max-width: 100%;
    justify-content: flex-start;
    padding-inline: 0;
  }
  .field .toggle-track {
    order: -1;
  }
  .toggle-track {
    display: flex;
    align-items: center;
    flex: none;
    width: 32px;
    height: 19px;
    padding: 3px;
    border-radius: 20px;
    background: var(--border);
    box-shadow: inset 0 0 0 1px var(--muted);
  }
  .toggle-track > span {
    width: 13px;
    height: 13px;
    border-radius: 50%;
    background: var(--text);
  }
  [aria-checked='true'] .toggle-track {
    background: var(--accent);
    box-shadow: none;
  }
  [aria-checked='true'] .toggle-track > span {
    transform: translateX(13px);
    background: var(--accentText);
  }
</style>
