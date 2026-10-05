<script lang="ts">
  import { uiError, type UiText, t, tr, locale } from './i18n';
  import MetadataFields from './MetadataFields.svelte';
  import Cover from './Cover.svelte';
  import { chooseCover } from './files';
  import type { Metadata } from './types';
  let {
    value,
    cover = $bindable(null),
    path = '',
    pageId,
  }: {
    value: Metadata;
    cover: string | null;
    path?: string;
    pageId?: string;
  } = $props();
  let error = $state<UiText>('');
  async function upload() {
    try {
      const selected = await chooseCover();
      if (selected) cover = selected;
    } catch (e) {
      error = uiError(e);
    }
  }
</script>

<div class="metadata-layout">
  <div>
    <Cover {path} {cover} {pageId} title={value.title || tr('Cover', $locale)} />
    <button title={t('m_06ff3625cd85', $locale)} onclick={upload}>{t('m_bb67353ac0fd', $locale)}</button>
    <button
      title={t('m_69867862f39e', $locale)}
      disabled={!cover}
      onclick={() => (cover = null)}>{t('m_3e56de69d525', $locale)}</button
    >
    {#if error}<p class="error-text" role="alert">{tr(error, $locale)}</p>{/if}
  </div>
  <div><MetadataFields {value} /></div>
</div>
