export type Point = { x: number; y: number };
export type CardBounds = { left: number; right: number; top: number; bottom: number };

/** Segment hit-testing avoids missing cards when mouse events skip over them. */
export function crossesCard(a: Point, b: Point, rect: CardBounds): boolean {
  let low = 0,
    high = 1;
  for (const [start, delta, min, max] of [
    [a.x, b.x - a.x, rect.left, rect.right],
    [a.y, b.y - a.y, rect.top, rect.bottom],
  ]) {
    if (!delta) {
      if (start < min || start > max) return false;
      continue;
    }
    const entry = (min - start) / delta,
      exit = (max - start) / delta;
    low = Math.max(low, Math.min(entry, exit));
    high = Math.min(high, Math.max(entry, exit));
    if (low > high) return false;
  }
  return true;
}

export class PageSweep {
  readonly before: string[];
  private readonly selected: Set<string>;
  private readonly visited = new Set<string>();
  private readonly adding: boolean;
  constructor(before: string[], first: string) {
    this.before = [...before];
    this.selected = new Set(before);
    this.adding = !this.selected.has(first);
    this.visit([first]);
  }
  visit(ids: Iterable<string>): string[] {
    for (const id of ids) {
      if (this.visited.has(id)) continue;
      this.visited.add(id);
      if (this.adding) this.selected.add(id);
      else this.selected.delete(id);
    }
    return [...this.selected];
  }
}

type Options = {
  selected: () => string[];
  change: (ids: string[]) => void;
  enabled: () => boolean;
  identity: () => string;
};

/** Mouse-only sweep; touch retains native scrolling and click selection. */
export function sweepSelection(node: HTMLElement, options: Options) {
  let pointer: number | null = null,
    first = '',
    identity = '';
  let start: Point = { x: 0, y: 0 },
    last = start,
    point = start;
  let sweep: PageSweep | null = null;
  let frame = 0,
    time = 0,
    suppressClick = false;
  let clickTimer: ReturnType<typeof setTimeout> | undefined;
  function finish(cancel: boolean) {
    if (cancel && sweep && identity === options.identity()) options.change(sweep.before);
    const id = pointer;
    pointer = null;
    sweep = null;
    cancelAnimationFrame(frame);
    frame = 0;
    node.classList.remove('sweep-selecting');
    if (id !== null && node.hasPointerCapture(id)) node.releasePointerCapture(id);
  }
  function draw(now: number) {
    frame = 0;
    if (pointer === null) return;
    if (!options.enabled() || identity !== options.identity()) {
      finish(true);
      return;
    }
    if (!sweep) return;
    const bounds = node.getBoundingClientRect();
    const elapsed = Math.min(32, Math.max(0, now - time));
    time = now;
    if (
      point.x >= bounds.left &&
      point.x <= bounds.right &&
      point.y >= bounds.top &&
      point.y <= bounds.bottom
    ) {
      const edge = Math.min(40, bounds.height / 4);
      const speed =
        point.y < bounds.top + edge
          ? -(bounds.top + edge - point.y) / edge
          : point.y > bounds.bottom - edge
            ? (point.y - bounds.bottom + edge) / edge
            : 0;
      node.scrollTop += speed * elapsed * 0.65;
    }
    const ids: string[] = [];
    for (const card of node.querySelectorAll<HTMLElement>('[data-selection-page]')) {
      const rect = card.getBoundingClientRect();
      const clipped = {
        left: Math.max(rect.left, bounds.left),
        right: Math.min(rect.right, bounds.right),
        top: Math.max(rect.top, bounds.top),
        bottom: Math.min(rect.bottom, bounds.bottom),
      };
      if (
        clipped.left <= clipped.right &&
        clipped.top <= clipped.bottom &&
        crossesCard(last, point, clipped)
      )
        ids.push(card.dataset.selectionPage!);
    }
    const next = sweep.visit(ids),
      previous = options.selected();
    if (next.length !== previous.length || next.some((id, index) => id !== previous[index]))
      options.change(next);
    last = point;
    frame = requestAnimationFrame(draw);
  }
  function down(event: PointerEvent) {
    if (
      event.pointerType !== 'mouse' ||
      event.button !== 0 ||
      !options.enabled() ||
      pointer !== null
    )
      return;
    const target = event.target as HTMLElement;
    if (target.closest('button, a, [data-reorder-handle]')) return;
    const card = target.closest<HTMLElement>('[data-selection-page]');
    if (!card || !node.contains(card)) return;
    suppressClick = false;
    clearTimeout(clickTimer);
    pointer = event.pointerId;
    first = card.dataset.selectionPage!;
    identity = options.identity();
    start = last = point = { x: event.clientX, y: event.clientY };
  }
  function move(event: PointerEvent) {
    if (event.pointerId !== pointer) return;
    point = { x: event.clientX, y: event.clientY };
    if (!options.enabled() || identity !== options.identity()) {
      finish(true);
      return;
    }
    if (!sweep && Math.hypot(point.x - start.x, point.y - start.y) >= 5) {
      sweep = new PageSweep(options.selected(), first);
      options.change(sweep.visit([]));
      suppressClick = true;
      clearTimeout(clickTimer);
      node.setPointerCapture(event.pointerId);
      node.classList.add('sweep-selecting');
      time = performance.now();
    }
    if (sweep) {
      event.preventDefault();
      if (!frame) frame = requestAnimationFrame(draw);
    }
  }
  function up(event: PointerEvent) {
    if (event.pointerId !== pointer) return;
    if (sweep) {
      point = { x: event.clientX, y: event.clientY };
      cancelAnimationFrame(frame);
      draw(performance.now());
    }
    finish(false);
    clickTimer = setTimeout(() => {
      suppressClick = false;
    }, 0);
  }
  function cancel() {
    finish(true);
  }
  function key(event: KeyboardEvent) {
    if (event.key === 'Escape' && pointer !== null) {
      event.preventDefault();
      event.stopImmediatePropagation();
      cancel();
    }
  }
  function click(event: MouseEvent) {
    if (suppressClick) {
      event.preventDefault();
      event.stopImmediatePropagation();
      suppressClick = false;
      return;
    }
    const target = event.target as HTMLElement;
    if (!options.enabled() || target.closest('input, label, button, a')) return;
    const card = target.closest<HTMLElement>('[data-selection-page]');
    if (!card || !node.contains(card)) return;
    const id = card.dataset.selectionPage!,
      selected = options.selected();
    options.change(
      selected.includes(id) ? selected.filter((value) => value !== id) : [...selected, id],
    );
  }
  function drag(event: DragEvent) {
    if (!(event.target as HTMLElement).closest('[data-reorder-handle]')) event.preventDefault();
  }
  node.addEventListener('pointerdown', down);
  node.addEventListener('click', click, true);
  node.addEventListener('dragstart', drag);
  node.addEventListener('lostpointercapture', cancel);
  window.addEventListener('pointermove', move, { passive: false });
  window.addEventListener('pointerup', up);
  window.addEventListener('pointercancel', cancel);
  window.addEventListener('blur', cancel);
  window.addEventListener('keydown', key, true);
  return {
    destroy() {
      finish(false);
      clearTimeout(clickTimer);
      node.removeEventListener('pointerdown', down);
      node.removeEventListener('click', click, true);
      node.removeEventListener('dragstart', drag);
      node.removeEventListener('lostpointercapture', cancel);
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', up);
      window.removeEventListener('pointercancel', cancel);
      window.removeEventListener('blur', cancel);
      window.removeEventListener('keydown', key, true);
    },
  };
}
