<script lang="ts">
  import { onMount, tick, untrack } from 'svelte';
  import { get } from 'svelte/store';
  import { locale, tr, t, number } from './i18n';
  import { visibleViewport, observeViewport } from './viewport';
  import type { Notifications } from './notifications';
  import NotificationCard from './NotificationCard.svelte';
  let { controller, reader = false }: { controller: Notifications; reader?: boolean } = $props();
  const notices = untrack(() => controller.state);
  let viewport = $state(visibleViewport());
  let topReserve = $state(16);
  let reserve = $state(20);
  let element: HTMLDivElement | undefined;
  let mounted = false;
  let previousFocus: HTMLElement | null = null;
  let announcement = $state('');
  let announcementSerial = 0;
  const available = $derived(Math.max(48, viewport.height - reserve - topReserve));
  $effect(() => {
    const reading = reader, view = viewport;
    let active = true, observer: ResizeObserver | undefined;
    if (!reading) { topReserve = 16; reserve = 20; }
    else void tick().then(() => {
      if (!active) return;
      const canvas = document.querySelector<HTMLElement>('.reader-canvas');
      const measure = () => {
        if (!active) return;
        topReserve = Math.max(16, (canvas?.getBoundingClientRect().top ?? view.top) - view.top + 8);
        const gutter = document.querySelector<HTMLElement>('.reader-progress-gutter');
        reserve = Math.max(20, view.bottom - (gutter?.getBoundingClientRect().top ?? view.bottom) + 8);
      };
      measure();
      if (canvas) { observer = new ResizeObserver(measure); observer.observe(canvas); }
    });
    return () => { active = false; observer?.disconnect(); };
  });
  $effect(() => {
    const next = $notices.announcement;
    if (!$notices.covered && next && next.serial !== announcementSerial) {
      announcementSerial = next.serial;
      // Locale switches redraw the cards, but do not repeat screen-reader announcements.
      const language = untrack(() => get(locale));
      announcement = controller.takeAnnouncements().map(notice => `${tr(notice.message, language)}${notice.repeats > 1 ? ` · ${t('notification.repeat', language, { count: number(notice.repeats, language) })}` : ''}`).join('\n');
    }
  });
  async function dismiss(id: string) {
    if (!mounted || !element) return;
    const card = element.querySelector<HTMLElement>(`[data-notice-id="${id}"]`);
    const focused = !!card?.contains(document.activeElement);
    controller.dismiss(id);
    if (!focused) return;
    await tick();
    if (!mounted || !element) return;
    const next = element.querySelector<HTMLElement>('.notification-close');
    const fallback = previousFocus?.isConnected && previousFocus !== document.body && previousFocus !== document.documentElement && !previousFocus.closest('[inert]') && !previousFocus.matches(':disabled') && previousFocus.getClientRects().length
      ? previousFocus : document.querySelector<HTMLElement>('[data-notification-return]');
    (next || fallback)?.focus({ preventScroll: true });
  }
  onMount(() => {
    mounted = true;
    const visibility = () => controller.cover('document', document.visibilityState !== 'visible');
    const hideWindow = () => controller.cover('window', true);
    const showWindow = () => controller.cover('window', false);
    const focus = (event: FocusEvent) => { if (element && event.target instanceof HTMLElement && !element.contains(event.target)) previousFocus = event.target; };
    previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const stopViewport = observeViewport(() => { viewport = visibleViewport(); });
    document.addEventListener('visibilitychange', visibility);
    document.addEventListener('focusin', focus);
    window.addEventListener('blur', hideWindow); window.addEventListener('focus', showWindow);
    visibility(); controller.cover('unmounted', false);
    return () => {
      mounted = false;
      controller.cover('unmounted', true);
      stopViewport(); document.removeEventListener('visibilitychange', visibility); document.removeEventListener('focusin', focus);
      window.removeEventListener('blur', hideWindow); window.removeEventListener('focus', showWindow);
    };
  });
</script>

<div bind:this={element} class="notification-host" hidden={$notices.covered} inert={$notices.covered} aria-hidden={$notices.covered}
  style:left={`${viewport.left + viewport.width / 2}px`}
  style:bottom={`calc(${Math.max(0, (typeof window === 'undefined' ? 0 : window.innerHeight) - viewport.bottom) + reserve}px + env(safe-area-inset-bottom))`}
  style:width={`${Math.min(750, Math.max(0, viewport.width - 24))}px`}
  style:--notification-height={`${available}px`} style:--notification-text-height={`${Math.max(40, Math.min(180, (available - 160) / 3))}px`}>
  <div class="notification-stack">
    {#each $notices.visible as notice (notice.id)}<NotificationCard {notice} {controller} covered={$notices.covered} onclose={dismiss} />{/each}
    {#if $notices.queued.length}<div class="notification-waiting" role="status">{t('notification.waiting', $locale, { count: number($notices.queued.length, $locale) })}</div>{/if}
  </div>
  <div class="sr-only notification-announcement" role="status" aria-live="polite" aria-atomic="true">{announcement}</div>
</div>

<style>
  .notification-host { position: fixed; transform: translateX(-50%); z-index: 45; pointer-events: none; }
  .notification-host[hidden] { display: none; }
  .notification-stack { display: flex; flex-direction: column; gap: 8px; max-height: var(--notification-height); overflow-y: auto; overscroll-behavior: contain; padding: 2px; }
  :global(.notification-card) { display: flex; align-items: flex-start; gap: 10px; flex: none; padding: 10px 12px; border: 1px solid var(--border); border-radius: 9px; background: var(--panel); color: var(--text); box-shadow: 0 6px 22px #0003; pointer-events: auto; animation: notice-arrive 140ms ease-out; }
  :global(.notification-icon) { flex: none; display: flex; margin-top: 8px; color: var(--accent); }
  :global(.notification-card[data-severity='warning'] .notification-icon) { color: var(--warning); }
  :global(.notification-card[data-severity='error'] .notification-icon) { color: var(--danger); }
  :global(.notification-card[data-severity='success'] .notification-icon) { color: var(--success); }
  :global(.notification-content) { flex: 1; min-width: 0; align-self: center; }
  :global(.notification-text) { max-height: var(--notification-text-height); overflow-y: auto; overscroll-behavior: contain; overflow-wrap: anywhere; white-space: pre-wrap; line-height: 1.45; padding: 2px; }
  :global(.notification-text:focus-visible) { outline: 2px solid var(--focus); outline-offset: -2px; }
  :global(.notification-repeat) { display: block; margin: 4px 2px 0; color: var(--muted); }
  :global(.notification-close) { flex: none; display: grid; place-items: center; width: 36px; height: 36px; min-height: 36px; padding: 0; border: 0; background: transparent; }
  :global([data-runtime='hosted'] .notification-close) { width: 44px; height: 44px; min-height: 44px; }
  .notification-waiting { align-self: center; flex: none; padding: 5px 12px; border: 1px solid var(--border); border-radius: 20px; background: var(--panel); color: var(--muted); font-size: .85em; pointer-events: auto; }
  .sr-only { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px; overflow: hidden; clip-path: inset(50%); white-space: nowrap; }
  @keyframes notice-arrive { from { opacity: 0; transform: translateY(4px); } to { opacity: 1; transform: translateY(0); } }
  @media (prefers-reduced-motion: reduce) { :global(.notification-card) { animation: none; } }
</style>
