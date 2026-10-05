<script lang="ts">
  import { onMount } from 'svelte';
  import { call, imageUrl } from './bridge';
  import type { Page } from './types';
  let { path, pageId, source }: { path: string; pageId: string; source?: Page['source'] } =
    $props();
  let holder: HTMLSpanElement;
  let src = $state('');
  onMount(() => {
    let live = true;
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          observer.disconnect();
          call(source ? 'source_thumbnail' : 'thumbnail', source ? { source } : { path, pageId })
            .then((v) => {
              if (live) src = imageUrl(v);
            })
            .catch(() => {});
        }
      },
      { rootMargin: '100px' },
    );
    observer.observe(holder);
    return () => {
      live = false;
      observer.disconnect();
    };
  });
</script>

<span class="thumbnail" bind:this={holder}
  >{#if src}<img {src} alt="" />{/if}</span
>

<style>
  .thumbnail {
    display: block;
    width: 30px;
    height: 42px;
    flex-shrink: 0;
    background: var(--raised);
    overflow: hidden;
    border-radius: 2px;
  }
  .thumbnail img {
    width: 100%;
    height: 100%;
    object-fit: contain;
  }
</style>
