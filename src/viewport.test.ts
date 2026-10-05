import { afterEach, describe, expect, it, vi } from 'vitest';
import { observeViewport, placeOverlay, visibleViewport } from './viewport';

afterEach(() => vi.unstubAllGlobals());
const anchor = (left: number, top: number, width = 100, height = 44) =>
  ({ left, top, right: left + width, bottom: top + height, width, height }) as DOMRect;
function viewport(width: number, height: number, left = 0, top = 0) {
  vi.stubGlobal('window', { innerWidth: 1024, innerHeight: 800,
    visualViewport: { width, height, offsetLeft: left, offsetTop: top } });
}

describe('visible viewport overlays', () => {
  it('uses the software keyboard / pinch viewport rather than the layout height', () => {
    viewport(320, 300, 40, 160);
    expect(visibleViewport()).toEqual({ left: 40, top: 160, width: 320, height: 300, right: 360, bottom: 460 });
    const result = placeOverlay(anchor(310, 370), 360, 250);
    expect(result.above).toBe(true);
    expect(result.left).toBe(48);
    expect(result.width).toBe(304);
    expect(result.top).toBeGreaterThanOrEqual(168);
    expect(result.top + result.maxHeight).toBeLessThanOrEqual(452);
  });
  it('places below when space permits and clamps the right edge', () => {
    viewport(390, 700);
    expect(placeOverlay(anchor(300, 50), 240, 200)).toEqual({ left: 142, top: 98, width: 240, maxHeight: 200, above: false });
  });
  it('caps tall panels to the available side without exceeding short landscape bounds', () => {
    viewport(844, 390);
    const result = placeOverlay(anchor(600, 170), 360, 540);
    expect(result).toEqual({ left: 476, top: 218, width: 360, maxHeight: 164, above: false });
  });
  it('falls back to layout dimensions without the visual viewport API', () => {
    vi.stubGlobal('window', { innerWidth: 500, innerHeight: 600 });
    expect(visibleViewport()).toEqual({ left: 0, top: 0, width: 500, height: 600, right: 500, bottom: 600 });
  });
  it('shares listeners and releases them when the final consumer closes', () => {
    const screen = new EventTarget(), view = new EventTarget();
    const add = vi.spyOn(screen, 'addEventListener'), remove = vi.spyOn(screen, 'removeEventListener');
    const viewAdd = vi.spyOn(view, 'addEventListener'), viewRemove = vi.spyOn(view, 'removeEventListener');
    vi.stubGlobal('window', Object.assign(screen, { visualViewport: view }));
    const first = vi.fn(), second = vi.fn();
    const closeFirst = observeViewport(first), closeSecond = observeViewport(second);
    expect(add).toHaveBeenCalledTimes(1); expect(viewAdd).toHaveBeenCalledTimes(2);
    view.dispatchEvent(new Event('resize'));
    expect(first).toHaveBeenCalledTimes(2); expect(second).toHaveBeenCalledTimes(2);
    closeFirst(); expect(remove).not.toHaveBeenCalled();
    view.dispatchEvent(new Event('scroll'));
    expect(first).toHaveBeenCalledTimes(2); expect(second).toHaveBeenCalledTimes(3);
    closeSecond(); expect(remove).toHaveBeenCalledTimes(1); expect(viewRemove).toHaveBeenCalledTimes(2);
  });
});
