import type { Page, Region } from './types';

export const handles = ['nw', 'n', 'ne', 'e', 'se', 's', 'sw', 'w'] as const;
export type Handle = (typeof handles)[number];
type Point = [number, number];
export type Gesture = {
  pageId: string;
  regionId: string | null;
  pointerId: number;
  action: 'draw' | 'move' | Handle;
  start: Point;
  clientStart: Point;
  original: number[];
  box: number[];
  width: number;
  height: number;
  meaningful: boolean;
};
const clamp = (n: number, lo: number, hi: number) => Math.max(lo, Math.min(n, hi));
export function imagePoint(
  client: Point,
  rect: { left: number; top: number; width: number; height: number },
  width: number,
  height: number,
): Point {
  return [
    ((client[0] - rect.left) * width) / rect.width,
    ((client[1] - rect.top) * height) / rect.height,
  ];
}
export function beginGesture(
  page: Page,
  region: Region | null,
  action: Gesture['action'],
  pointerId: number,
  point: Point,
  client: Point,
): Gesture {
  const start: Point = [clamp(point[0], 0, page.width), clamp(point[1], 0, page.height)];
  const box = region ? [...region.bbox] : [...start, ...start];
  return {
    pageId: page.id,
    regionId: region?.id || null,
    pointerId,
    action,
    start,
    clientStart: client,
    original: box,
    box,
    width: page.width,
    height: page.height,
    meaningful: false,
  };
}
export function moveBox(box: number[], dx: number, dy: number, width: number, height: number) {
  dx = clamp(dx, -box[0], width - box[2]);
  dy = clamp(dy, -box[1], height - box[3]);
  return [box[0] + dx, box[1] + dy, box[2] + dx, box[3] + dy];
}
export function updateGesture(g: Gesture, point: Point, client: Point): Gesture {
  let box = [...g.original];
  const [x, y] = [clamp(point[0], 0, g.width), clamp(point[1], 0, g.height)];
  if (g.action === 'draw')
    box = [
      Math.min(x, g.start[0]),
      Math.min(y, g.start[1]),
      Math.max(x, g.start[0]),
      Math.max(y, g.start[1]),
    ];
  else if (g.action === 'move')
    box = moveBox(box, point[0] - g.start[0], point[1] - g.start[1], g.width, g.height);
  else {
    const dx = point[0] - g.start[0],
      dy = point[1] - g.start[1];
    if (g.action.includes('w')) box[0] = clamp(box[0] + dx, 0, box[2] - 1);
    if (g.action.includes('e')) box[2] = clamp(box[2] + dx, box[0] + 1, g.width);
    if (g.action.includes('n')) box[1] = clamp(box[1] + dy, 0, box[3] - 1);
    if (g.action.includes('s')) box[3] = clamp(box[3] + dy, box[1] + 1, g.height);
  }
  return {
    ...g,
    box,
    meaningful: Math.hypot(client[0] - g.clientStart[0], client[1] - g.clientStart[1]) >= 3,
  };
}
export function contains(bubble: number[], box: number[]) {
  return bubble[0] <= box[0] && bubble[1] <= box[1] && bubble[2] >= box[2] && bubble[3] >= box[3];
}
export function setRegionBox(region: Region, box: number[], width: number, height: number) {
  if (box.length !== 4 || !box.every(Number.isFinite) || box[2] <= box[0] || box[3] <= box[1])
    return false;
  const next = [
    clamp(box[0], 0, width - 1),
    clamp(box[1], 0, height - 1),
    clamp(box[2], 1, width),
    clamp(box[3], 1, height),
  ];
  if (next[2] <= next[0] || next[3] <= next[1]) return false;
  region.bbox = next;
  if (region.bubble && !contains(region.bubble, next)) region.bubble = null;
  return true;
}
export type GeometryEdit = {
  pageId: string;
  regionId: string;
  before: Pick<Region, 'bbox' | 'bubble'> | null;
  after: Pick<Region, 'bbox' | 'bubble'>;
  created?: Region;
  index: number;
};
export function geometry(region: Region) {
  return structuredClone({ bbox: region.bbox, bubble: region.bubble });
}
/** Only geometry is replaced; concurrent OCR/text/style updates remain owned by their fields. */
export function applyGeometryEdit(page: Page, edit: GeometryEdit, back: boolean) {
  if (page.id !== edit.pageId) return false;
  const i = page.regions.findIndex((r) => r.id === edit.regionId);
  if (!edit.before) {
    if (back) {
      if (i < 0) return false;
      edit.created = structuredClone(page.regions[i]);
      page.regions.splice(i, 1);
    } else {
      if (i >= 0 || !edit.created) return false;
      page.regions.splice(
        Math.min(edit.index, page.regions.length),
        0,
        structuredClone(edit.created),
      );
    }
  } else {
    if (i < 0) return false;
    Object.assign(page.regions[i], structuredClone(back ? edit.before : edit.after));
  }
  return true;
}
