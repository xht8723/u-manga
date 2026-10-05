<script lang="ts">
  import { t, tr, locale } from './i18n';
  import ComboBox from './ComboBox.svelte';
  import type { Metadata } from './types';
  let { value }: { value: Metadata } = $props();
</script>

<label>{t('m_7e8cd2056da7', $locale)}<input bind:value={value.title} required maxlength="240" /></label>
<div class="form-grid">
  <label>{t('m_88447b83090c', $locale)}<input bind:value={value.creator} /></label><label
    >{t('m_088b23e3327d', $locale)}<ComboBox
      label={tr('Source language', $locale)}
      bind:value={value.language}
      options={[
        { value: '', label: 'Use default', disabled: false },
        ...[
          ['ja', 'Japanese'],
          ['zh-Hans', 'Chinese (Simplified)'],
          ['zh-Hant', 'Chinese (Traditional)'],
          ['en', 'English'],
          ['ko', 'Korean'],
          ['fr', 'French'],
          ['de', 'German'],
          ['es', 'Spanish'],
          ['ar', 'Arabic'],
          ['hi', 'Hindi'],
          ['ru', 'Russian'],
          ['th', 'Thai'],
          ['vi', 'Vietnamese'],
          ['ta', 'Tamil'],
        ].flatMap(([id, name]) => [{ value: id, label: String(name).trim(), disabled: false }]),
      ]}
    /></label
  >
</div>
<label>{t('m_526e0087cc3f', $locale)}<textarea rows="3" bind:value={value.description}></textarea></label>
<label
  >{t('m_1331275bc537', $locale)}<input
    value={value.tags.join(', ')}
    placeholder={t('m_07263c31ab8b', $locale)}
    onchange={(e) =>
      (value.tags = [
        ...new Set(
          e.currentTarget.value
            .split(',')
            .map((t) => t.trim())
            .filter(Boolean),
        ),
      ])}
  /></label
>
