<script lang="ts">
  import { uiError, type UiText, t, tr, locale } from './i18n';
  import ComboBox from './ComboBox.svelte';
  import Modal from './Modal.svelte';
  import { open, save } from '@tauri-apps/plugin-dialog';
  import { call, native } from './bridge';
  let {
    path,
    title,
    chapterId = null,
    onclose,
    oncomplete,
  }: {
    path: string;
    title: string;
    chapterId?: string | null;
    onclose: () => void;
    oncomplete: () => void;
  } = $props();
  let format = $state('cbz'),
    working = $state(false),
    error = $state<UiText>('');
  async function exportPages() {
    working = true;
    error = '';
    try {
      const selected =
        format === 'images'
          ? await open({ directory: true, title: tr('Choose export folder') })
          : await save({
              defaultPath: `${title.replace(/[<>:"/\\|?*]/g, '_')}.${format}`,
              filters: [{ name: format.toUpperCase(), extensions: [format] }],
            });
      if (typeof selected !== 'string') return;
      await call('book_export', { path, chapterId, destination: selected, format });
      oncomplete();
    } catch (e) {
      error = uiError(e);
    } finally {
      working = false;
    }
  }
</script>

<Modal
  title={tr(`Export · ${title}`, $locale)}
  onclose={() => {
    if (!working) onclose();
  }}
>
  <label
    >{t('m_2f343666aaa8', $locale)}<ComboBox
      label={tr('Format', $locale)}
      bind:value={format}
      disabled={working}
      options={[
        { value: 'cbz', label: 'CBZ archive', disabled: false },
        { value: 'pdf', label: 'PDF', disabled: false },
        { value: 'images', label: 'PNG images', disabled: false },
      ]}
    /></label
  >
  {#if !native}<p>{t('m_62cfa28a76e4', $locale)}</p>{/if}
  {#if error}<p class="error-text" role="alert">{tr(error, $locale)}</p>{/if}
  {#snippet footer()}<button disabled={working} onclick={onclose}>{t('m_19766ed6ccb2', $locale)}</button>
    <button
      class="primary"
      title={t('m_bb9908f966dc', $locale)}
      disabled={working || !native}
      onclick={exportPages}>{working ? t('m_a2dd9cbb7013', $locale) : t('m_5853e22f6085', $locale)}</button
    >
  {/snippet}
</Modal>
