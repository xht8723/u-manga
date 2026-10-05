export interface VisibleViewport {
  left: number;
  top: number;
  width: number;
  height: number;
  right: number;
  bottom: number;
}

/** Coordinates match fixed-position overlays and getBoundingClientRect(), including pinch zoom. */
export function visibleViewport(): VisibleViewport {
  const view = typeof window === 'undefined' ? undefined : window.visualViewport;
  const left = view?.offsetLeft ?? 0, top = view?.offsetTop ?? 0;
  const width = view?.width ?? (typeof window === 'undefined' ? 0 : window.innerWidth);
  const height = view?.height ?? (typeof window === 'undefined' ? 0 : window.innerHeight);
  return { left, top, width, height, right: left + width, bottom: top + height };
}

const listeners = new Set<() => void>();
let unlisten: (() => void) | undefined;
/** Share resize/keyboard/pinch listeners without changing document styles. */
export function observeViewport(callback: () => void): () => void {
  listeners.add(callback);
  if (!unlisten && typeof window !== 'undefined') {
    const view = window.visualViewport;
    const notify = () => { for (const listener of [...listeners]) listener(); };
    window.addEventListener('resize', notify);
    view?.addEventListener('resize', notify);
    view?.addEventListener('scroll', notify);
    unlisten = () => {
      window.removeEventListener('resize', notify);
      view?.removeEventListener('resize', notify);
      view?.removeEventListener('scroll', notify);
    };
  }
  callback();
  return () => {
    listeners.delete(callback);
    if (!listeners.size) { unlisten?.(); unlisten = undefined; }
  };
}

/** Fit an anchored overlay into the visible screen; callers may right-align its anchor. */
export function placeOverlay(anchor: DOMRect, panelWidth: number, desiredHeight: number, gap = 4) {
  const view = visibleViewport(), margin = 8;
  const width = Math.max(0, Math.min(panelWidth, view.width - margin * 2));
  const below = Math.max(0, view.bottom - margin - anchor.bottom - gap);
  const before = Math.max(0, anchor.top - gap - view.top - margin);
  const above = below < desiredHeight && before > below;
  const available = Math.min(above ? before : below, Math.max(0, view.height - margin * 2));
  const maxHeight = Math.max(0, Math.min(desiredHeight, available));
  const left = Math.max(view.left + margin, Math.min(anchor.left, view.right - width - margin));
  const top = Math.max(view.top + margin, Math.min(
    above ? anchor.top - gap - maxHeight : anchor.bottom + gap,
    view.bottom - maxHeight - margin,
  ));
  return { left, top, width, maxHeight, above };
}
