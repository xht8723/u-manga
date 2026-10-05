<script lang="ts">
  import { onMount } from 'svelte';
  import { call, imageUrl, hosted } from './bridge';
  let {
    path,
    cover,
    pageId,
    title,
  }: { path: string; cover?: string | null; pageId?: string | null; title: string } = $props();
  let holder: HTMLDivElement;
  let near = $state(false);
  let src = $state('');
  let failed = $state(false);
  $effect(() => {
    const custom = cover;
    const page = pageId;
    const p = path;
    let live = true;
    failed = false;
    if (near) {
      if (custom || page) {
        (custom && (custom.startsWith('data:') || hosted)
          ? Promise.resolve(custom)
          : custom
            ? call('source_thumbnail', {
                source: { path: custom, kind: 'image', entry: null, index: 0 },
              })
            : call('thumbnail', { path: p, pageId: page! })
        )
          .then((v) => {
            if (live) src = imageUrl(v);
          })
          .catch(() => {
            if (live) failed = true;
          });
      } else src = '';
    }
    return () => {
      live = false;
    };
  });
  onMount(() => {
    const o = new IntersectionObserver(
      (es) => {
        if (es.some((e) => e.isIntersecting)) {
          near = true;
          o.disconnect();
        }
      },
      { rootMargin: '300px' },
    );
    o.observe(holder);
    return () => o.disconnect();
  });
</script>

<div bind:this={holder} class="cover-art">
  {#if src && !failed}<img {src} alt="" loading="lazy" onerror={() => (failed = true)} />{:else}<div
      class="cover-placeholder"
    >
      <span>U</span><b>{title}</b>
    </div>{/if}
</div>
