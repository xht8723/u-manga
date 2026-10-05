import { describe, expect, it } from 'vitest';
import { get } from 'svelte/store';
import en from '../resources/i18n/en.json';
import zh from '../resources/i18n/zh-Hans.json';
import {
  count,
  locale,
  resolveLocale,
  setLanguage,
  tr,
  uiJoin,
  sameUiText,
  updateSystemLanguage,
  type MessageKey,
} from './i18n';
import { settingsDefaults } from './book-state';
import { readinessSignature, testSignature } from './setup-state';

const key = (text: string) => Object.entries(en).find(([, v]) => v === text)![0] as MessageKey;
describe('interface localization', () => {
  it('compares complete error content, not message references or active locale', () => {
    const first = { key: key('Unable to complete this operation.'), args: {}, fallback: 'network', detail: 'Network diagnostic\r\nendpoint unavailable' };
    const copy = JSON.parse(JSON.stringify(first));
    expect(first === copy).toBe(false);
    for (const language of ['en', 'zh-Hans'] as const) {
      setLanguage(language);
      expect(sameUiText(first, copy)).toBe(true);
      expect(sameUiText(first, { ...copy, detail: 'Different diagnostic' })).toBe(false);
      expect(sameUiText(first, tr(copy, 'en').replace(/\r\n/g, '\n'))).toBe(true);
      expect(sameUiText(first, null)).toBe(false);
    }
    const a = { key: key('{arg0} regions saved'), args: { arg0: 3 }, fallback: '3 regions saved' };
    expect(sameUiText(a, { ...a, args: { arg0: 4 } })).toBe(false);
    expect(sameUiText(uiJoin([a, first]), uiJoin([structuredClone(a), copy]))).toBe(true);
    setLanguage('en');
  });
  it('resolves all Chinese display locales and otherwise falls back to English', () => {
    for (const tag of ['zh-CN', 'zh-TW', 'zh-Hant-HK', 'zh_Hans_SG', 'zh'])
      expect(resolveLocale('system', tag)).toBe('zh-Hans');
    for (const tag of ['en-US', 'ja-JP', 'fr-FR', '', 'invalid'])
      expect(resolveLocale('system', tag)).toBe('en');
    expect(resolveLocale('en', 'zh-CN')).toBe('en');
    expect(resolveLocale('zh-Hans', 'en-US')).toBe('zh-Hans');
  });
  it('respects an override when the system changes, and follows it only in system mode', () => {
    setLanguage('en', 'en-US');
    updateSystemLanguage('zh-TW');
    expect(get(locale)).toBe('en');
    setLanguage('system');
    expect(get(locale)).toBe('zh-Hans');
    updateSystemLanguage('fr-FR');
    expect(get(locale)).toBe('en');
  });
  it('has complete catalogs and matching parameter names', () => {
    expect(tr('Region 2 / 4', 'zh-Hans')).toBe('区域 2 / 4');
    expect(tr('Region 2 / 4', 'en')).toBe('Region 2 / 4');
    expect(Object.keys(zh).sort()).toEqual(Object.keys(en).sort());
    for (const k of Object.keys(en) as MessageKey[]) {
      expect(zh[k].trim(), k).not.toBe('');
      const placeholders = (s: string) => [...s.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();
      expect(placeholders(zh[k]), en[k]).toEqual(placeholders(en[k]));
    }
  });
  it('switches saved messages, joined errors, and parameterized progress without mutation', () => {
    const source = {
      key: key('{arg0} regions saved'),
      args: { arg0: 8 },
      fallback: '8 regions saved',
    };
    const joined = uiJoin([
      source,
      { key: key('Provider returned no text'), args: {}, fallback: 'Provider returned no text' },
    ]);
    const before = JSON.stringify(joined);
    expect(tr(joined, 'zh-Hans')).toBe('已保存 8 个区域\n服务未返回文字');
    expect(tr(joined, 'en')).toBe('8 regions saved\nProvider returned no text');
    expect(JSON.stringify(joined)).toBe(before);
    expect(tr('Batch 2 / 4 · 6 regions', 'zh-Hans')).toBe('批次 2 / 4 · 6 个区域');
  });
  it('formats plural counts and preserves unknown external details and placeholder values', () => {
    expect(count(1, key('{count} book'), key('{count} books'), 'en')).toBe('1 book');
    expect(count(2, key('{count} book'), key('{count} books'), 'en')).toBe('2 books');
    expect(count(2, key('{count} book'), key('{count} books'), 'zh-Hans')).toBe('2 本书');
    expect(tr('Remote XYZ: {{private}}', 'zh-Hans')).toBe('Remote XYZ: {{private}}');
    expect(tr('Open Library', 'zh-Hans')).toBe('打开 Library');
    expect(tr({ key: 'future-key', args: {}, fallback: 'Unknown future message' }, 'zh-Hans')).toBe(
      'Unknown future message',
    );
  });
  it('does not change processing settings or readiness/service-test identities', () => {
    const s = settingsDefaults(),
      before = JSON.stringify(s.translation);
    const signatures = [readinessSignature(s), testSignature(s)];
    s.uiLanguage = 'zh-Hans';
    setLanguage(s.uiLanguage);
    expect([readinessSignature(s), testSignature(s)]).toEqual(signatures);
    expect(JSON.stringify(s.translation)).toBe(before);
    setLanguage('system', 'en');
  });
});
