import { MediaQuery } from 'svelte/reactivity';
import { hosted } from './runtime-mode';

export const compactMedia = '(max-width: 800px), (max-width: 1024px) and (max-height: 500px)';
// All consumers share one media subscription; no component changes global classes.
const compactQuery = new MediaQuery(compactMedia, false);

export function useCompactLayout(): { readonly current: boolean } {
  return { get current() { return hosted && compactQuery.current; } };
}
