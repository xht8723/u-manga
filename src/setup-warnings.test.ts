import { describe, it, expect } from 'vitest';
import { distinctWarnings, unseenWarnings } from './setup-warnings';
import { uiError } from './i18n';

describe('visible setup warnings', () => {
  it('deduplicates native messages and field errors in both languages', () => {
    const message = 'Prompts exceed the 16 KiB UTF-8 limit.';
    for (const language of ['en', 'zh-Hans'] as const) {
      expect(distinctWarnings([message, uiError(message), ''], language)).toHaveLength(1);
      expect(unseenWarnings([uiError(message)], [message], language)).toEqual([]);
      expect(unseenWarnings([message], [], language)).toEqual([message]);
    }
  });
  it('retains distinct operation failures and readiness problems', () => {
    const missing = 'Download or verify Manga OCR Japanese in Local models.';
    const operation = 'Files are missing or damaged. Use Download / resume to repair this pack.';
    expect(unseenWarnings([missing, operation], [missing], 'en')).toEqual([operation]);
    expect(unseenWarnings([missing, operation], [], 'en')).toEqual([missing, operation]);
  });
});
