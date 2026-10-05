<script lang="ts">
  import ComboBox from './ComboBox.svelte';
  import { t, locale, type LanguagePreference } from './i18n';
  let {
    value,
    onchange,
    disabled = false,
  }: {
    value: LanguagePreference;
    onchange: (value: LanguagePreference) => void;
    disabled?: boolean;
  } = $props();
  let selected = $derived(value);
</script>

<label class="interface-language">
  <span>{t('m_8027f432a7fb', $locale)}</span>
  <ComboBox
    label="Interface language"
    value={selected}
    {disabled}
    options={[
      { value: 'system', label: 'Follow system' },
      { value: 'en', label: 'English', literal: true },
      { value: 'zh-Hans', label: '简体中文', literal: true },
    ]}
    oncommit={(next) => onchange(next as LanguagePreference)}
  />
</label>

<style>
  .interface-language {
    display: grid;
    gap: 7px;
    margin-bottom: 20px;
  }
</style>
