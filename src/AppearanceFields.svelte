<script lang="ts">
  import { t, tr, locale } from './i18n';
  import ComboBox from './ComboBox.svelte';
  import { untrack } from 'svelte';
  import type { AppearanceSettings, ThemeProfile } from './types';
  import { profile } from './appearance';
  let {
    value,
    fonts,
    onpreview,
    onapply,
    oncancel,
  }: {
    value: AppearanceSettings;
    fonts: string[];
    onpreview: (a: AppearanceSettings) => void;
    onapply: (a: AppearanceSettings) => void;
    oncancel: () => void;
  } = $props();
  let draft = $state<AppearanceSettings>(untrack(() => structuredClone($state.snapshot(value))));
  let tab = $state<'day' | 'night'>(untrack(() => value.active));
  let section = $state('Interface');
  let p = $derived(draft[tab]);
  function preview() {
    onpreview(structuredClone($state.snapshot(draft)));
  }
  function reset(all = false) {
    let d = profile(tab === 'night');
    if (all) draft[tab] = d;
    else if (section === 'Interface') {
      draft[tab] = { ...p, colors: d.colors, font: d.font, scale: 1, density: 1 };
    } else if (section === 'Reader') {
      draft[tab] = { ...p, readerBackground: d.readerBackground, margin: d.margin, gap: d.gap };
    } else draft[tab].filters = d.filters;
    preview();
  }
  const labels: Record<string, string> = {
    window: 'Window',
    panel: 'Panels',
    raised: 'Raised surfaces',
    text: 'Text',
    muted: 'Secondary text',
    accent: 'Accent',
    accentText: 'Text on accent',
    border: 'Borders',
    selection: 'Selection',
    focus: 'Focus ring',
    danger: 'Error',
    warning: 'Warning',
    success: 'Success',
  };
</script>

<div class="tabs">
  <button
    title={t('m_bddc2ce0bca2', $locale)}
    class:active={tab === 'day'}
    onclick={() => {
      tab = 'day';
      draft.active = 'day';
      preview();
    }}>{t('m_d4078adde572', $locale)}</button
  ><button
    title={t('m_6c0cda9e4f44', $locale)}
    class:active={tab === 'night'}
    onclick={() => {
      tab = 'night';
      draft.active = 'night';
      preview();
    }}>{t('m_4791fd6db68d', $locale)}</button
  ><span class="spacer"></span><small>{t('m_f65ddc1ff37b', $locale)}</small>
</div>
<div class="tabs small">
  {#each ['Interface', 'Reader', 'Page filters'] as name}<button
      class:active={section === name}
      onclick={() => (section = name)}>{tr(name, $locale)}</button
    >{/each}
</div>
<div class="modal-body" oninput={preview} onchange={preview}>
  {#if section === 'Interface'}<div class="color-grid">
      {#each Object.keys(labels) as key}<label
          ><span>{tr(labels[key], $locale)}</span><input
            type="color"
            bind:value={p.colors[key]}
            aria-label={tr(labels[key], $locale)}
          /></label
        >{/each}
    </div>
    <label
      >{t('m_ad55b73eb9e0', $locale)}<ComboBox
        label={t('m_ad55b73eb9e0', $locale)}
        editable
        bind:value={p.font}
        options={fonts.map((f) => ({ value: f, label: f, literal: true }))}
        oninput={preview}
      /></label
    >
    <label
      >{t('m_4d496f9120b4', $locale)} <output>{Math.round(p.scale * 100)}%</output><input
        type="range"
        min="0.85"
        max="1.6"
        step="0.05"
        bind:value={p.scale}
      /></label
    >
    <label
      >{t('m_77a283d69258', $locale)} <output>{p.density.toFixed(2)}</output><input
        type="range"
        min="0.7"
        max="1.5"
        step="0.05"
        bind:value={p.density}
      /></label
    >
  {:else if section === 'Reader'}<label
      >{t('m_03fe531de419', $locale)}<input type="color" bind:value={p.readerBackground} /></label
    ><label
      >{t('m_57b3d81b7150', $locale)} <output>{p.margin}px</output><input
        type="range"
        min="0"
        max="100"
        bind:value={p.margin}
      /></label
    ><label
      >{t('m_f8c58a826e1d', $locale)} <output>{p.gap}px</output><input
        type="range"
        min="0"
        max="100"
        bind:value={p.gap}
      /></label
    >
  {:else}<p class="notice"> {t('m_f97ca5e8a03f', $locale)} </p>
    {#each [{ key: 'brightness', label: 'Brightness', min: 0.2, max: 1.5 }, { key: 'contrast', label: 'Contrast', min: 0.3, max: 2 }, { key: 'warmth', label: 'Warmth', min: 0, max: 1 }, { key: 'saturation', label: 'Saturation', min: 0, max: 2 }, { key: 'grayscale', label: 'Grayscale', min: 0, max: 1 }, { key: 'inversion', label: 'Inversion', min: 0, max: 1 }] as f}<label
        >{tr(f.label, $locale)}<output>{p.filters[f.key as keyof ThemeProfile['filters']].toFixed(2)}</output
        ><input
          type="range"
          min={f.min}
          max={f.max}
          step="0.05"
          bind:value={p.filters[f.key as keyof ThemeProfile['filters']]}
        /></label
      >{/each}{/if}
</div>
<footer class="safe-controls">
  <button
    title={tr(`Reset ${section.toLowerCase()} settings for the ${tab} profile`, $locale)}
    onclick={() => reset()}>{t('m_8e4b7f18830e', $locale)}</button
  ><button title={tr(`Reset all settings in the ${tab} profile`, $locale)} onclick={() => reset(true)}
    >{t('m_daee7606b339', $locale)} {tr(tab, $locale)}</button
  >
</footer>
