<script lang="ts">
  import { t, tr, locale } from './i18n';
  import { onMount, tick, type Snippet } from 'svelte';
  import { modalIdentity, modalStack, registerModal } from './modal-stack';
  import { hosted } from './runtime-mode';
  import { useCompactLayout } from './mobile-layout.svelte';
  import { observeViewport, visibleViewport } from './viewport';
  const compact = useCompactLayout();
  let viewport = $state(visibleViewport());
  let {
    title,
    children,
    footer,
    headerActions,
    onclose,
    wide = false,
    stretch = false,
    inactive = false,
    closable = true,
    closeDisabled = false,
    guided = false,
    scrollKey,
    className = '',
    portal = false,
    returnFocus,
  }: {
    title: string;
    children: Snippet;
    footer?: Snippet;
    headerActions?: Snippet;
    onclose: () => void;
    wide?: boolean;
    stretch?: boolean;
    inactive?: boolean;
    closable?: boolean;
    closeDisabled?: boolean;
    guided?: boolean;
    scrollKey?: string;
    className?: string;
    portal?: boolean;
    returnFocus?: HTMLElement;
  } = $props();
  const owner = modalIdentity();
  const rank = $derived($modalStack.indexOf(owner));
  const top = $derived(rank < 0 || $modalStack.at(-1) === owner);
  let element: HTMLDivElement;
  let body: HTMLDivElement;
  // A dialog opened inside another dialog must escape its parent's inert surface.
  function mountSurface(node: HTMLDivElement) {
    if (!portal) return;
    document.body.appendChild(node);
    return { destroy: () => node.remove() };
  }
  $effect(() => {
    if (scrollKey !== undefined) body?.scrollTo({ top: 0 });
  });
  onMount(() => {
    const before = returnFocus || document.activeElement as HTMLElement;
    const remove = registerModal(owner);
    const stopViewport = hosted ? observeViewport(() => { viewport = visibleViewport(); }) : undefined;
    void tick().then(() => { if (top && !inactive) element.focus({ preventScroll: true }); });
    return () => {
      stopViewport?.();
      remove();
      void tick().then(() => {
        if (before?.isConnected && !before.closest('[inert]') && !before.matches(':disabled'))
          before.focus({ preventScroll: true });
        else document.querySelector<HTMLElement>(
          `[data-modal-owner="${$modalStack.at(-1)}"]`,
        )?.focus({ preventScroll: true });
      });
    };
  });
  function key(e: KeyboardEvent) {
    if (!top || inactive) return;
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      if (closable && !closeDisabled) onclose();
    }
    if (e.key === 'Tab') {
      const items = Array.from(
        element.querySelectorAll<HTMLElement>(
          'button:not(:disabled),input:not(:disabled),select:not(:disabled),textarea:not(:disabled),[tabindex="0"]',
        ),
      ).filter((el) => el.offsetParent !== null);
      const first = items[0],
        last = items.at(-1);
      if (!first) {
        e.preventDefault();
        element.focus({ preventScroll: true });
      } else if (e.shiftKey && (document.activeElement === first || document.activeElement === element)) {
        e.preventDefault();
        last?.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first?.focus();
      }
    }
  }
</script>

<div use:mountSurface class="scrim" class:compact={compact.current} style:z-index={60 + Math.max(0, rank)}
  style:left={hosted ? `${viewport.left}px` : undefined}
  style:top={hosted ? `${viewport.top}px` : undefined}
  style:width={hosted ? `${viewport.width}px` : undefined}
  style:height={hosted ? `${viewport.height}px` : undefined}
  inert={inactive || !top} aria-hidden={inactive || !top}>
  <div
    bind:this={element}
    data-modal-owner={owner}
    class={`modal library-modal ${className}`}
    class:wide
    class:stretch
    class:guided
    style:max-height={hosted ? `${Math.max(0, viewport.height - (compact.current ? 16 : 60))}px` : undefined}
    role="dialog"
    aria-modal="true"
    aria-label={title}
    tabindex="-1"
    onkeydown={key}
  >
    <header>
      <h2>{title}</h2>
      {#if headerActions}{@render headerActions()}{/if}
      {#if closable}<button
          disabled={closeDisabled}
          title={t('m_4c8516492497', $locale)}
          aria-label={t('m_c8df66c5fe3f', $locale)}
          onclick={onclose}>✕</button
        >{/if}
    </header>
    <div class="modal-body" bind:this={body}>{@render children()}</div>
    {#if footer}<footer>{@render footer()}</footer>{/if}
  </div>
</div>

<style>
  .scrim.compact {
    padding: 8px;
    padding-top: max(8px, env(safe-area-inset-top));
    padding-bottom: max(8px, env(safe-area-inset-bottom));
  }
  .compact > .modal { max-width: 100%; }
  .compact > .modal > header { flex: none; padding: 14px 16px; }
  .compact > .modal > header h2 { min-width: 0; overflow-wrap: anywhere; }
  .compact .modal-body { min-height: 0; padding: 8px 16px 16px; overscroll-behavior: contain; }
  .compact > .modal > footer { flex: none; padding: 12px 16px; justify-content: flex-end; flex-wrap: wrap; }
</style>
