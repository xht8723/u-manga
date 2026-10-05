import { get, writable } from 'svelte/store';
import en from '../resources/i18n/en.json';
import zh from '../resources/i18n/zh-Hans.json';
import patterns from '../resources/i18n/patterns.json';

export type LanguagePreference = 'system' | 'en' | 'zh-Hans';
export type Locale = 'en' | 'zh-Hans';
export type MessageKey = keyof typeof en;
export type UiMessage = {
  key: string;
  args: Record<string, string | number>;
  fallback: string;
  detail?: string;
  parts?: UiText[];
  separator?: string;
};
export type UiText = string | UiMessage;
export const locale = writable<Locale>('en');
let preference: LanguagePreference = 'system';
let systemLanguage = typeof navigator !== 'undefined' ? navigator.language : 'en';
export function resolveLocale(value: LanguagePreference, system = systemLanguage): Locale {
  return value === 'system' ? (/^zh(?:[-_]|$)/i.test(system) ? 'zh-Hans' : 'en') : value;
}
export function setLanguage(value: LanguagePreference, system = systemLanguage) {
  preference = value;
  systemLanguage = system;
  const resolved = resolveLocale(value, system);
  locale.set(resolved);
  if (typeof document !== 'undefined') document.documentElement.lang = resolved;
}
export function updateSystemLanguage(system: string) {
  setLanguage(preference, system);
}
export function languagePreference() {
  return preference;
}
export const languageOptions = [
  { value: 'system' as const, label: 'Follow system' },
  { value: 'en' as const, label: 'English' },
  { value: 'zh-Hans' as const, label: '简体中文' },
];
const byEnglish = new Map(Object.entries(en).map(([key, value]) => [value, key]));
export function t(
  key: MessageKey,
  language: Locale = get(locale),
  args: Record<string, string | number> = {},
): string {
  const pattern = (language === 'zh-Hans' ? zh[key] : en[key]) || en[key] || key;
  // Only explicitly identified application labels may be localized inside parameters.
  // Book titles, model IDs, paths, source text and provider diagnostics remain verbatim.
  const labelSlots: Record<string, string[]> = {
    '{arg0} recognized regions saved · {arg1}': ['arg1'],
    'Step {arg0} of {arg1}: {arg2}': ['arg2'],
  };
  return pattern.replace(/\{(\w+)\}/g, (full, name) => {
    const value = args[name] ?? full;
    return labelSlots[en[key]]?.includes(name) ? tr(value, language) : String(value);
  });
}
export function isMessage(value: unknown): value is UiMessage {
  return (
    !!value && typeof value === 'object' && 'key' in value && 'fallback' in value && 'args' in value
  );
}
/** Locale is applied only at presentation; raw data and captured processing settings stay untouched. */
export function tr(value: unknown, language: Locale = get(locale)): string {
  if (value == null) return '';
  if (Array.isArray(value))
    return value
      .map((v) => tr(v, language))
      .filter(Boolean)
      .join('\n');
  if (isMessage(value)) {
    if (value.parts) return value.parts.map((p) => tr(p, language)).join(value.separator ?? '\n');
    const text =
      value.key in en
        ? t(value.key as MessageKey, language, value.args)
        : tr(value.fallback, language);
    return value.detail ? `${text}\n${value.detail}` : text;
  }
  const text = value instanceof Error ? value.message : String(value);
  const key = byEnglish.get(text);
  if (key) return t(key as MessageKey, language);
  if (text.includes('\n'))
    return text
      .split('\n')
      .map((line) => tr(line, language))
      .join('\n');
  for (const pattern of patterns) {
    if (!text.startsWith(pattern.parts[0])) continue;
    let rest = text.slice(pattern.parts[0].length);
    const args: Record<string, string> = {};
    let matches = true;
    for (let i = 0; i < pattern.slots.length; i++) {
      const end = pattern.parts[i + 1];
      const index = !end
        ? rest.length
        : i + 1 === pattern.slots.length
          ? rest.endsWith(end)
            ? rest.length - end.length
            : -1
          : rest.indexOf(end);
      if (index < 0) {
        matches = false;
        break;
      }
      args[pattern.slots[i]] = rest.slice(0, index);
      rest = rest.slice(index + end.length);
    }
    if (matches && !rest) return t(pattern.key as MessageKey, language, args);
  }
  return text;
}
export function rawText(value: unknown): string {
  return isMessage(value)
    ? value.fallback
    : value instanceof Error
      ? value.message
      : String(value ?? '');
}
/** Compare full displayed content independently of locale and object identity.
 * Keep diagnostics: two generic summaries with different details are not duplicates.
 */
export function sameUiText(left: UiText | null | undefined, right: UiText | null | undefined): boolean {
  if (!left || !right) return false;
  return tr(left, 'en').replace(/\r\n?/g, '\n') === tr(right, 'en').replace(/\r\n?/g, '\n');
}
export function uiError(value: unknown): UiText {
  if (isMessage(value) && value.key !== 'diagnostic') return value;
  const raw = rawText(value);
  if (byEnglish.has(raw) || tr(raw, 'zh-Hans') !== raw) return raw;
  return {
    key: byEnglish.get('Unable to complete this operation.')!,
    args: {},
    fallback: raw,
    detail: raw,
  };
}
export function uiJoin(parts: UiText[], separator = '\n'): UiText {
  const present = parts.filter((p) => rawText(p));
  if (!present.length) return '';
  if (present.length === 1) return present[0];
  return {
    key: 'joined',
    args: {},
    fallback: present.map(rawText).join(separator),
    parts: present,
    separator,
  };
}
export function number(
  value: number,
  language: Locale = get(locale),
  options?: Intl.NumberFormatOptions,
) {
  return new Intl.NumberFormat(language, options).format(value);
}
export function count(
  value: number,
  one: MessageKey,
  many: MessageKey,
  language: Locale = get(locale),
) {
  return t(new Intl.PluralRules(language).select(value) === 'one' ? one : many, language, {
    count: number(value, language),
  });
}
