<script lang="ts">
  import { onMount } from 'svelte';
  import { CircleCheck, Info, TriangleAlert, CircleX, X } from 'lucide-svelte';
  import { t, tr, locale, number } from './i18n';
  import type { Notice, Notifications } from './notifications';
  let { notice, controller, covered, onclose }: { notice: Notice; controller: Notifications; covered: boolean; onclose: (id: string) => void } = $props();
  let element: HTMLElement | undefined;
  let mounted = $state(false);
  let touchInterface = false;
  const touches = new Set<number>();
  function release(event: PointerEvent) {
    if (touches.delete(event.pointerId)) controller.pause(notice.id, `touch-${event.pointerId}`, false);
  }
  onMount(() => {
    mounted = true;
    window.addEventListener('pointerup', release, true);
    window.addEventListener('pointercancel', release, true);
    return () => {
      mounted = false;
      window.removeEventListener('pointerup', release, true);
      window.removeEventListener('pointercancel', release, true);
    };
  });
  $effect(() => {
    if (!mounted || !element) return;
    controller.pause(notice.id, 'hover', !covered && !touchInterface && element.matches(':hover'));
    controller.pause(notice.id, 'focus', !covered && element.contains(document.activeElement));
  });
  function focusOut(event: FocusEvent) {
    if (!element || element.contains(event.relatedTarget as Node | null)) return;
    const id = notice.id;
    queueMicrotask(() => {
      if (mounted && element) controller.pause(id, 'focus', element.contains(document.activeElement));
    });
  }
  function key(event: KeyboardEvent) {
    // A focused notice owns its keys; arrows must not turn the manga page.
    event.stopPropagation();
    if (event.key === 'Escape') { event.preventDefault(); onclose(notice.id); }
    const text = (event.target as HTMLElement).closest<HTMLElement>('.notification-text');
    if (text) {
      const movement: Record<string, number> = { ArrowDown: 24, ArrowUp: -24, PageDown: text.clientHeight, PageUp: -text.clientHeight };
      if (event.key in movement || event.key === 'Home' || event.key === 'End') {
        event.preventDefault();
        text.scrollTop = event.key === 'Home' ? 0 : event.key === 'End' ? text.scrollHeight : text.scrollTop + movement[event.key];
      }
    }
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions (Keyboard focus belongs to the close button.) -->
<section bind:this={element} class="notification-card" data-notice-id={notice.id} data-severity={notice.severity}
  aria-label={t(`notification.${notice.severity}`, $locale)}
  onpointerenter={event => { touchInterface = event.pointerType === 'touch'; if (!touchInterface) controller.pause(notice.id, 'hover', true); }}
  onpointerleave={event => { if (event.pointerType !== 'touch') controller.pause(notice.id, 'hover', false); }}
  onpointerdown={event => { if (event.pointerType === 'touch') { touches.add(event.pointerId); controller.pause(notice.id, `touch-${event.pointerId}`, true); } }}
  onlostpointercapture={release}
  onfocusin={() => controller.pause(notice.id, 'focus', true)} onfocusout={focusOut} onkeydown={key}>
  <span class="notification-icon" aria-hidden="true">
    {#if notice.severity === 'success'}<CircleCheck size={19} />{:else if notice.severity === 'warning'}<TriangleAlert size={19} />{:else if notice.severity === 'error'}<CircleX size={19} />{:else}<Info size={19} />{/if}
  </span>
  <div class="notification-content">
    <!-- svelte-ignore a11y_no_noninteractive_tabindex (Scrollable diagnostics need keyboard focus.) -->
    <div class="notification-text" role="region" aria-label={t('notification.content', $locale)} tabindex="0">{tr(notice.message, $locale)}</div>
    {#if notice.repeats > 1}<small class="notification-repeat">{t('notification.repeat', $locale, { count: number(notice.repeats, $locale) })}</small>{/if}
  </div>
  <button class="notification-close" aria-label={t('notification.dismiss', $locale)} title={t('notification.dismiss', $locale)} onclick={() => onclose(notice.id)}><X size={18} /></button>
</section>
