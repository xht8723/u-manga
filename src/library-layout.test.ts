import { expect, it } from 'vitest';
import { libraryGeometry } from './library-layout';
it('fits two compact covers at small phone widths including the scrollbar', () => {
  const layout = libraryGeometry(281, 'grid', 1, true);
  expect(layout.columns).toBe(2);
  expect(layout.cardWidth * 2 + layout.gap).toBe(281);
  expect(layout.rowHeight - layout.gap - layout.cardWidth * 1.48).toBeGreaterThanOrEqual(130);
});
it('uses one column for large text and retains desktop geometry', () => {
  expect(libraryGeometry(281, 'grid', 1.6, true).columns).toBe(1);
  expect(libraryGeometry(362, 'grid', 1, false).columns).toBe(1);
  expect(libraryGeometry(768, 'grid', 1, false).columns).toBe(3);
  expect(libraryGeometry(800, 'list', 1.6, true).columns).toBe(1);
});
