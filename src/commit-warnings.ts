import { sameUiText, uiJoin, type UiText } from './i18n';

// Keep the receipt intact while presenting its message through the normal UI-text path.
export function warningMessage(warning: unknown): UiText {
  if (
    warning && typeof warning === 'object' && 'id' in warning &&
    typeof warning.id === 'string' && 'message' in warning
  ) return warning.message as UiText;
  return warning as UiText;
}

/** One receipt produces one notice; all distinct diagnostics remain readable. */
export function warningGroup(warnings: readonly unknown[]): UiText {
  const messages: UiText[] = [];
  for (const warning of warnings) {
    const message = warningMessage(warning);
    if (message && !messages.some(previous => sameUiText(previous, message))) messages.push(message);
  }
  if (!messages.length) return '';
  return uiJoin([{ key: 'notification.commitWarnings', args: {}, fallback: 'Saved with warnings.' }, ...messages]);
}
