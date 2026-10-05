<script lang="ts">
  import { t, tr, locale } from './i18n';
  import { onMount } from 'svelte';
  import type { Page } from './types';
  import { call, imageUrl, hosted } from './bridge';
  let {
    page,
    path,
    translated,
    filter,
    width,
    onvisible,
    onerror,
  }: {
    page: Page;
    path: string;
    translated: boolean;
    filter: string;
    width: number;
    onvisible?: (id: string) => void;
    onerror: (e: unknown) => void;
  } = $props();
  let element: HTMLDivElement;
  let near = $state(false);
  let src = $state('');
  let token = 0;
  let displayedIdentity = '';
  $effect(() => {
    const p = page;
    const t = translated;
    const n = near;
    const version = p.revision;
    const root = path;
    const request = ++token;
    const identity = `${root}/${p.id}/${t}`;
    if (!hosted || !n || displayedIdentity !== identity) src = '';
    displayedIdentity = identity;
    if (n) {
      call('page_image', { path: root, pageId: p.id, translated: t })
        .then((v) => {
          if (request === token) src = imageUrl(v);
        })
        .catch((error) => { if (request === token) onerror(error); });
    }
    return () => { if (request === token) token++; };
  });
  onMount(() => {
    const o = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          near = e.isIntersecting;
        }
      },
      { rootMargin: '800px' },
    );
    const v = new IntersectionObserver(
      (entries) => {
        for (const e of entries) if (e.isIntersecting) onvisible?.(page.id);
      },
      { rootMargin: '-45% 0px -45% 0px', threshold: 0 },
    );
    o.observe(element);
    v.observe(element);
    return () => {
      token++;
      o.disconnect();
      v.disconnect();
    };
  });
</script>

<div
  class="paper"
  bind:this={element}
  style:width={`${width}px`}
  style:aspect-ratio={`${page.width}/${page.height}`}
  data-page={page.id}
>
  {#if src}<img {src} alt={tr(`Page ${page.number + 1}: ${page.name}`, $locale)} style:filter />{:else}<span
      class="page-loading">{t('m_0a30a815d67d', $locale)} {page.number + 1}</span
    >{/if}
</div>
