<script lang="ts">
  import { t, tr, locale } from './i18n';
  import Modal from './Modal.svelte';
  import BookDetailsFields from './BookDetailsFields.svelte';
  import Organizer from './Organizer.svelte';
  import { metadata, emptyPreview } from './book-state';
  import { call } from './bridge';
  import type { Book, ImportPreview } from './types';
  let {
    onclose,
    oncreated,
    scale = 1,
  }: { scale?: number; onclose: () => void; oncreated: (b: Book) => void } = $props();
  let details = $state(metadata()),
    preview = $state(emptyPreview()),
    cover = $state<string | null>(null),
    step = $state(1),
    automatic = $state(true);
  async function create(draft: ImportPreview) {
    const b = await call('book_create', {
      metadata: $state.snapshot(details),
      preview: draft,
      cover,
    });
    oncreated(b);
  }
</script>

{#if step === 1}
  <Modal title={t('m_db6973ab7e64', $locale)} {onclose} wide>
    <BookDetailsFields value={details} bind:cover />
    {#snippet footer()}<button onclick={onclose}>{t('m_19766ed6ccb2', $locale)}</button><button
        class="primary"
        title={t('m_b948a6c0c1c4', $locale)}
        disabled={!details.title.trim()}
        onclick={() => (step = 2)}>{t('m_1b79e669655c', $locale)}</button
      >{/snippet}
  </Modal>
{:else}
  <Organizer
    {scale}
    initial={preview}
    bind:automatic
    {onclose}
    oncreate={create}
    onback={(draft) => {
      preview = draft;
      step = 1;
    }}
  />
{/if}
