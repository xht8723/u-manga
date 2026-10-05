<script lang="ts">
  import { type UiText, t, tr, locale } from './i18n';
  import { onMount, tick } from 'svelte';
  import { useCompactLayout } from './mobile-layout.svelte';
  import { observeViewport, placeOverlay } from './viewport';
  const compact = useCompactLayout();
  import {
    ArrowLeft,
    ChevronLeft,
    ChevronRight,
    MoreHorizontal,
    PanelTopClose,
    Sun,
    Moon,
    Maximize,
    Minimize,
    PenLine,
    Download,
    Settings2,
  } from 'lucide-svelte';
  import PageViewSelector from './PageViewSelector.svelte';
  import ComboBox from './ComboBox.svelte';
  import Modal from './Modal.svelte';
  import { hosted } from './runtime-mode';
  import ToggleSwitch from './ToggleSwitch.svelte';
  let {
    index,
    total,
    continuous = $bindable(),
    zoom = $bindable(),
    translated,
    translating,
    translationReason = '',
    showPages,
    night,
    fullscreen,
    onback,
    onnavigate,
    onview,
    ontranslation,
    oncollapse,
    onpages,
    ontheme,
    onfullscreen,
    oneditor,
    onexport,
    onsettings,
  }: {
    index: number;
    total: number;
    continuous: boolean;
    zoom: number;
    translated: boolean;
    translating: boolean;
    translationReason?: UiText;
    showPages: boolean;
    night: boolean;
    fullscreen: boolean;
    onback: () => void;
    onnavigate: (index: number) => void;
    onview: (translated: boolean) => void;
    ontranslation: (enabled: boolean) => void;
    oncollapse: () => void;
    onpages: (visible: boolean) => void;
    ontheme: () => void;
    onfullscreen: () => void;
    oneditor: () => void;
    onexport: () => void;
    onsettings: () => void;
  } = $props();
  let more = $state(false),
    jump = $state(false),
    target = $state(1);
  let root: HTMLDivElement;
  let trigger = $state<HTMLButtonElement>();
  let panel = $state<HTMLDivElement>();
  let bounds = $state({ left: 0, top: 0, width: 350, maxHeight: 450, above: false });
  const supportsFullscreen = !hosted || (typeof document !== 'undefined' && document.fullscreenEnabled);
  const settingsHelp = $derived(hosted ? tr('Configure missing models or services on the PC.', $locale) : tr('Open Settings from More.', $locale));
  function position() { if (trigger && panel) bounds = placeOverlay(trigger.getBoundingClientRect(), 350, panel.scrollHeight); }
  async function toggleMore() { more = !more; if (more) { await tick(); position(); panel?.focus({ preventScroll: true }); } }
  function action(fn: () => void) {
    more = false;
    fn();
  }
  function key(e: KeyboardEvent) {
    if (e.key === 'Escape' && more) {
      e.stopPropagation();
      e.preventDefault();
      more = false;
      root.querySelector<HTMLButtonElement>('[data-reader-options]')?.focus();
    }
  }
  onMount(() => {
    const stopViewport = observeViewport(position);
    const outside = (e: PointerEvent) => {
      if (!root.contains(e.target as Node) && !(e.target as Element)?.closest('.combo-popup'))
        more = false;
    };
    document.addEventListener('pointerdown', outside, true);
    return () => { stopViewport(); document.removeEventListener('pointerdown', outside, true); };
  });
</script>

<div class="reader-controls" class:compact-reader={compact.current} bind:this={root} onkeydown={key} role="presentation">
  <div class="reader-toolbar" role="toolbar" aria-label={t('m_65eae975696e', $locale)}>
    <button class="reader-back" aria-label={t('m_42d2d0b686bc', $locale)} title={t('m_42d2d0b686bc', $locale)} onclick={onback}
      ><ArrowLeft size={18} /></button
    >
    <span class="reader-tool-spacer"></span>
    <button class="reader-previous"
      aria-label={t('m_1208ec01f223', $locale)}
      title={t('m_999d92689a61', $locale)}
      disabled={index === 0}
      onclick={() => onnavigate(index - 1)}><ChevronLeft size={18} /></button
    >
    <button
      class="page-counter"
      aria-label={t('m_7ecbecebed62', $locale)}
      title={t('m_7ecbecebed62', $locale)}
      onclick={() => {
        target = index + 1;
        jump = true;
        more = false;
      }}>{index + 1} / {total}</button
    >
    <button class="reader-next"
      aria-label={t('m_c08ac736a5e2', $locale)}
      title={t('m_c521f73a0b20', $locale)}
      disabled={index === total - 1}
      onclick={() => onnavigate(index + 1)}><ChevronRight size={18} /></button
    >
    <PageViewSelector {translated} onchange={onview} />
    <span class="reader-tool-spacer"></span>
    <span
      class="reader-translation-control"
      title={tr(!translating && translationReason
        ? `${tr(translationReason, $locale)} ${settingsHelp}`
        : undefined, $locale)}
    >
      <ToggleSwitch
        label={t('m_ae4c3ff8a1c2', $locale)}
        focusableWhenDisabled
        checked={translating}
        disabled={!translating && !!translationReason}
        title={tr(translating
          ? 'Stop adding reading jobs; existing jobs continue'
          : translationReason ||
            'Translate the visible page and prefetch the next two pages in this chapter', $locale)}
        onchange={ontranslation}
      />
      {#if !translating && translationReason}<span class="reader-translation-help" role="tooltip"
          >{tr(translationReason, $locale)} {settingsHelp}</span
        >{/if}
    </span>
    {#if !compact.current}<button
      aria-label={tr(night ? 'Switch to Day mode' : 'Switch to Night mode', $locale)}
      title={tr(night ? 'Switch to Day mode' : 'Switch to Night mode', $locale)}
      onclick={ontheme}
      >{#if night}<Moon size={18} />{:else}<Sun size={18} />{/if}</button
    >
    <button aria-label={t('m_936a161c3931', $locale)} title={t('m_a9029e19d5b3', $locale)} onclick={() => action(oneditor)}
      ><PenLine size={18} /></button
    >{/if}
    <button class="reader-more" bind:this={trigger}
      aria-label={t('m_4dfcb8909732', $locale)}
      title={t('m_4dfcb8909732', $locale)}
      aria-expanded={more}
      data-reader-options onclick={() => void toggleMore()}><MoreHorizontal size={19} /></button
    >
    {#if !compact.current}<button
      aria-label={t('m_04bd649c1845', $locale)}
      title={t('m_dac0a826df3f', $locale)}
      onclick={oncollapse}><PanelTopClose size={18} /></button
    >{/if}
  </div>
  {#if more}<div
      class="reader-options"
      bind:this={panel}
      style:position="fixed" style:left={`${bounds.left}px`} style:top={`${bounds.top}px`}
      style:width={`${bounds.width}px`} style:max-height={`${bounds.maxHeight}px`}
      role="dialog"
      tabindex="-1"
      aria-label={t('m_4dfcb8909732', $locale)}
      data-reader-panel
    >
      {#if compact.current}<div class="reader-option-actions">
        <button onclick={() => action(oneditor)}><PenLine size={18} />{t('m_936a161c3931', $locale)}</button>
        <button onclick={ontheme}>{#if night}<Moon size={18} />{:else}<Sun size={18} />{/if}{tr(night ? 'Switch to Day mode' : 'Switch to Night mode', $locale)}</button>
        <button onclick={() => action(oncollapse)}><PanelTopClose size={18} />{t('m_04bd649c1845', $locale)}</button>
      </div>{/if}
      <ToggleSwitch
        label={t('m_d82fd62f9623', $locale)}
        checked={showPages}
        title={tr(showPages ? 'Hide the page thumbnail strip' : 'Show the page thumbnail strip', $locale)}
        wide
        onchange={onpages}
      />
      <label
        >{t('m_d9eea85d6813', $locale)}<ComboBox
          label={t('m_d9eea85d6813', $locale)}
          bind:value={continuous}
          options={[
            { value: false, label: 'Paged' },
            { value: true, label: 'Continuous' },
          ]}
        /></label
      >
      <label
        >{t('m_8bc067ef2d85', $locale)} {zoom}%<input
          aria-label={t('m_509c517ede79', $locale)}
          type="range"
          min="25"
          max="160"
          step="5"
          bind:value={zoom}
        /></label
      >
      {#if supportsFullscreen}<div class="reader-option-actions">
        <button
          title={tr(fullscreen ? 'Exit fullscreen (F11)' : 'Enter fullscreen (F11)', $locale)}
          onclick={onfullscreen}
          >{#if fullscreen}<Minimize size={16} />{:else}<Maximize size={16} />{/if}{fullscreen
            ? t('m_37fd4e355ba3', $locale)
            : t('m_c461dbb2bab7', $locale)} <small>F11</small></button
        >
      </div>{/if}
      <div class="reader-option-actions secondary">
        {#if !hosted}<button title={t('m_9896c50bd4d0', $locale)} onclick={() => action(onexport)}
          ><Download size={16} />{t('m_a2d04370b02a', $locale)}</button
        >{/if}
        <button title={t('m_21f3effd0926', $locale)} onclick={() => action(onsettings)}
          ><Settings2 size={16} />{t('m_74a883a037bc', $locale)}</button
        >
      </div>
    </div>{/if}
</div>
{#if jump}<Modal title={t('m_7ecbecebed62', $locale)} onclose={() => (jump = false)}>
    <form
      onsubmit={(e) => {
        e.preventDefault();
        if (Number.isInteger(target) && target >= 1 && target <= total) {
          jump = false;
          onnavigate(target - 1);
        }
      }}
    >
      <label
        >{t('m_7d2fb09b9236', $locale)}{total})<input
          aria-label={t('m_05fa2e3b0a10', $locale)}
          type="number"
          min="1"
          max={total}
          step="1"
          bind:value={target}
        /></label
      >
      <button
        class="primary"
        type="submit"
        disabled={!Number.isInteger(target) || target < 1 || target > total}>{t('m_6cc8519b9158', $locale)}</button
      >
    </form>
  </Modal>{/if}
