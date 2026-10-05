import { tr, type UiText, type Locale } from './i18n';

export function warningKey(message: UiText, language: Locale): string {
  return tr(message, language).replace(/\s+/g, ' ').trim();
}
export function distinctWarnings(messages: UiText[], language: Locale): UiText[] {
  const seen = new Set<string>();
  return messages.filter((message) => {
    const key = warningKey(message, language);
    return !!key && !seen.has(key) && !!seen.add(key);
  });
}
export function unseenWarnings(messages: UiText[], visible: UiText[], language: Locale): UiText[] {
  const shown = new Set(visible.map((message) => warningKey(message, language)));
  return distinctWarnings(messages, language).filter(
    (message) => !shown.has(warningKey(message, language)),
  );
}
