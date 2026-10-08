import { sameUiText, type UiText } from './i18n';

/** Bulk controls report one cause with its affected jobs, independently of locale. */
export function groupJobErrors(errors: { id: string; message: UiText }[]) {
  const groups: { message: UiText; ids: string[] }[] = [];
  for (const error of errors) {
    const group = groups.find((value) => sameUiText(value.message, error.message));
    if (group) {
      if (!group.ids.includes(error.id)) group.ids.push(error.id);
    } else groups.push({ message: error.message, ids: [error.id] });
  }
  return groups;
}
