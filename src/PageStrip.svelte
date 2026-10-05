<script lang="ts">
  import { t, tr, locale } from './i18n';
  import Thumbnail from './Thumbnail.svelte';
  import { untrack, onMount } from 'svelte';
  import type { Page } from './types';
  let {
    pages,
    path,
    index,
    onnavigate,
  }: { pages: Page[]; path: string; index: number; onnavigate: (i: number) => void } = $props();
  let scroll = $state(0),
    width = $state(800);
  let start = $derived(Math.max(0, Math.floor(scroll / 80) - 2));
  let end = $derived(Math.min(pages.length, start + Math.ceil(width / 80) + 4));
  let viewport: HTMLDivElement;
  // Vertical mouse wheels should browse this horizontal strip. Native horizontal
  // trackpad/Shift-wheel events are consumed once, never added to deltaY twice.
  onMount(() => {
    const wheel = (event: WheelEvent) => {
      if (event.ctrlKey || event.metaKey || viewport.scrollWidth <= viewport.clientWidth) return;
      const unit = event.deltaMode === 1 ? 24 : event.deltaMode === 2 ? viewport.clientWidth : 1;
      const delta =
        (Math.abs(event.deltaX) >= Math.abs(event.deltaY) ? event.deltaX : event.deltaY) * unit;
      if (!delta) return;
      event.preventDefault();
      event.stopPropagation();
      viewport.scrollLeft += delta;
    };
    viewport.addEventListener('wheel', wheel, { passive: false });
    return () => viewport.removeEventListener('wheel', wheel);
  });
  function key(event: KeyboardEvent) {
    if (event.ctrlKey || event.metaKey || event.altKey) return;
    const delta: Record<string, number> = {
      ArrowLeft: -80,
      ArrowRight: 80,
      PageUp: -viewport.clientWidth,
      PageDown: viewport.clientWidth,
    };
    if (!(event.key in delta) && !['Home', 'End'].includes(event.key)) return;
    event.preventDefault();
    event.stopPropagation();
    viewport.focus({ preventScroll: true });
    viewport.scrollLeft =
      event.key === 'Home'
        ? 0
        : event.key === 'End'
          ? viewport.scrollWidth
          : viewport.scrollLeft + delta[event.key];
  }
  $effect(() => {
    const left = index * 80;
    const visibleWidth = width;
    untrack(() => {
      if (!viewport) return;
      if (left < viewport.scrollLeft) viewport.scrollLeft = left;
      else if (left + 80 > viewport.scrollLeft + visibleWidth)
        viewport.scrollLeft = left + 80 - visibleWidth;
      scroll = viewport.scrollLeft;
    });
  });
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions (The scroll region must be keyboard reachable independently of page selection.) -->
<div
  class="page-strip"
  bind:this={viewport}
  role="navigation"
  aria-label={t('m_d82fd62f9623', $locale)}
  tabindex="0"
  onkeydown={key}
  bind:clientWidth={width}
  onscroll={(e) => (scroll = e.currentTarget.scrollLeft)}
>
  <div style:width={`${pages.length * 80}px`} class="strip-inner">
    <div style:left={`${start * 80}px`} class="strip-items">
      {#each pages.slice(start, end) as p, j (p.id)}<button
          class:current={index === start + j}
          aria-label={tr(`Go to page ${start + j + 1}`, $locale)}
          aria-current={index === start + j ? 'page' : undefined}
          title={tr(`Go to page ${start + j + 1}`, $locale)}
          onclick={() => onnavigate(start + j)}
          ><Thumbnail {path} pageId={p.id} /><span>{start + j + 1}</span></button
        >{/each}
    </div>
  </div>
</div>
