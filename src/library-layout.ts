export function libraryGeometry(
  width: number,
  view: 'grid' | 'card' | 'list',
  scale: number,
  compact = false,
) {
  const uiScale = Math.max(1, scale);
  const gap = compact ? 12 : view === 'grid' ? 24 : view === 'card' ? 16 : 8;
  const minimum = view === 'grid' ? (compact ? 130 : 190) : 390;
  const columns =
    view === 'list' ? 1 : Math.max(1, Math.floor((width + gap) / (minimum * uiScale + gap)));
  const cardWidth = Math.max(1, (width - gap * (columns - 1)) / columns);
  const rowHeight =
    view === 'grid'
      ? cardWidth * 1.48 + (compact ? 152 : 122) * uiScale
      : (view === 'card' ? 210 : compact ? 144 : 108) * uiScale;
  return { gap, columns, cardWidth, rowHeight, uiScale };
}
