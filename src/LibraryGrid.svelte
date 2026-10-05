<script lang="ts">
  import { t, tr, locale } from './i18n';
  import type { BookSummary } from './types';
  import Cover from './Cover.svelte';
  import { libraryGeometry } from './library-layout';
  let {
    books,
    view = 'grid',
    scale = 1,
    compact = false,
    filterKey = '',
    onopen,
    onmenu,
  }: {
    books: BookSummary[];
    view?: 'grid' | 'card' | 'list';
    scale?: number;
    compact?: boolean;
    filterKey?: string;
    onopen: (b: BookSummary) => void;
    onmenu: (b: BookSummary, e: MouseEvent) => void;
  } = $props();
  let width = $state(1000),
    height = $state(800),
    scroll = $state(0);
  let measuredScale = $state(1);
  const geometry = $derived(libraryGeometry(width, view, Math.max(scale, measuredScale), compact));
  const gap = $derived(geometry.gap),
    uiScale = $derived(geometry.uiScale),
    columns = $derived(geometry.columns),
    rowHeight = $derived(geometry.rowHeight);
  function measure(node: HTMLElement) {
    const update = () => {
      measuredScale = parseFloat(getComputedStyle(node).fontSize) / 13;
    };
    const observer = new MutationObserver(update);
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ['style', 'data-appearance'],
    });
    update();
    return { destroy: () => observer.disconnect() };
  }
  let viewport = $state<HTMLDivElement>();
  $effect(() => {
    filterKey;
    view;
    scale;
    if (viewport) {
      viewport.scrollTop = 0;
      scroll = 0;
    }
  });
  let start = $derived(Math.max(0, Math.floor(scroll / rowHeight) - 2) * columns);
  let end = $derived(
    Math.min(books.length, (Math.ceil((scroll + height) / rowHeight) + 2) * columns),
  );
  let total = $derived(Math.ceil(books.length / columns) * rowHeight);
</script>

<div
  class={`virtual-library library-${view}`}
  use:measure
  style:--library-scale={uiScale}
  bind:this={viewport}
  bind:clientWidth={width}
  bind:clientHeight={height}
  onscroll={(e) => (scroll = e.currentTarget.scrollTop)}
>
  <div style:height={`${total}px`} class="grid-spacer">
    <div
      class="cover-grid"
      style:gap={`${gap}px`}
      style:grid-template-columns={`repeat(${columns},minmax(0,1fr))`}
      style:transform={`translateY(${Math.floor(start / columns) * rowHeight}px)`}
    >
      {#each books.slice(start, end) as b (b.id)}<article
          class="book-card"
          style:height={`${rowHeight - gap}px`}
          oncontextmenu={(e) => {
            e.preventDefault();
            onmenu(b, e);
          }}
        >
          <button
            class="cover-button"
            onclick={() => onopen(b)}
            title={tr(`Open ${b.metadata.title}`, $locale)}
            aria-label={tr(`Open ${b.metadata.title}`, $locale)}
            ><Cover
              path={b.path}
              cover={b.cover}
              pageId={b.coverPage}
              title={b.metadata.title}
            /></button
          >
          <div class="book-card-info">
            <div class="card-title">
              <button
                title={tr(`View chapters in ${b.metadata.title}`, $locale)}
                onclick={() => onopen(b)}>{b.metadata.title}</button
              >{#if view !== 'list'}<button
                  class="ellipsis"
                  title={t('m_077f42456a7a', $locale)}
                  aria-label={tr(`Options for ${b.metadata.title}`, $locale)}
                  aria-haspopup="menu"
                  onclick={(e) => onmenu(b, e)}>•••</button
                >{/if}
            </div>
            <small class="creator">{b.metadata.creator || '—'}</small>
            <div class="card-status">
              <span>{tr(`${b.completed}/${b.chapters} read`, $locale)}</span><span
                >{b.review
                  ? tr(`${b.review} review`, $locale)
                  : b.translated
                    ? tr(`${b.translated}/${b.pages} translated`, $locale)
                    : t('m_d41ef2ce8711', $locale)}</span
              >
            </div>
            {#if view === 'card'}<p class="card-description">{b.metadata.description}</p>
              <div class="card-tags">
                {#each b.metadata.tags.slice(0, 3) as tag}<span class="tag">{tag}</span>{/each}
              </div>{/if}
            <progress
              aria-label={t('m_9ee7937a77b6', $locale)}
              value={b.completed}
              max={Math.max(1, b.chapters)}
            ></progress>
          </div>
          {#if view === 'list'}<button
              class="ellipsis list-options"
              title={t('m_077f42456a7a', $locale)}
              aria-label={tr(`Options for ${b.metadata.title}`, $locale)}
              aria-haspopup="menu"
              onclick={(e) => onmenu(b, e)}>•••</button
            >{/if}
        </article>{/each}
    </div>
  </div>
</div>
