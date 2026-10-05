import { describe, it, expect } from 'vitest';
import {
  emptyGlossary,
  normalizeGlossary,
  entryErrors,
  mergeTerms,
  parseGlossary,
  encodeGlossary,
  MAX_GLOSSARY_ENTRIES,
  MAX_GLOSSARY_BYTES,
  MAX_GLOSSARY_FILE_BYTES,
} from './glossary';
import { effectiveSettings, metadata, settingsDefaults } from './book-state';
import type { Book } from './types';
describe('book glossary', () => {
  it('round trips Unicode, quoted delimiters, BOM, and multiline terms', () => {
    const rows = [{ source: '魔王,"A"\n城\t門', target: '魔王、城门' }];
    for (const format of ['csv', 'tsv'])
      expect(parseGlossary(encodeGlossary(rows, format), format)).toEqual(rows);
    expect(parseGlossary(' source , target \r\n 勇者 , 勇者 ', 'csv')).toEqual([
      { source: '勇者', target: '勇者' },
    ]);
    expect(parseGlossary('猫\t貓', 'tsv')).toHaveLength(1);
  });
  it('rejects invalid quoting, missing fields, duplicates, oversized files', () => {
    for (const text of ['a,b,c', '"a,b', '"a"x,b', 'a,', 'a"x,b'])
      expect(() => parseGlossary(text, 'csv')).toThrow();
    expect(() => parseGlossary('a'.repeat(4 * 1024 * 1024 + 1), 'csv')).toThrow();
    expect(
      entryErrors([
        { source: 'a', target: 'b' },
        { source: ' a ', target: 'c' },
      ]).every(Boolean),
    ).toBe(true);
    expect(() =>
      normalizeGlossary({ ...emptyGlossary(), entries: [{ source: '', target: '' }] }),
    ).toThrow();
  });
  it('round trips accepted raw content with encoded quote and row overhead', () => {
    const plain = Array.from({ length: MAX_GLOSSARY_ENTRIES }, (_, i) => ({
      source: 'k' + String(i).padStart(5, '0') + 's'.repeat(203),
      target: 't'.repeat(209),
    }));
    const quotes = [{ source: 'A', target: '"'.repeat(MAX_GLOSSARY_BYTES - 1) }];
    for (const rows of [plain, quotes]) {
      for (const format of ['csv', 'tsv']) {
        const encoded = encodeGlossary(rows, format);
        expect(new TextEncoder().encode(encoded).length).toBeLessThanOrEqual(
          MAX_GLOSSARY_FILE_BYTES,
        );
        expect(parseGlossary(encoded, format)).toEqual(rows);
      }
    }
  });
  it('bounds rows and encoded files while retaining the raw limit for duplicate input', () => {
    expect(() => parseGlossary('a,b\n'.repeat(MAX_GLOSSARY_ENTRIES + 1), 'csv')).toThrow('10,000');
    expect(() => parseGlossary('a,' + 'b'.repeat(MAX_GLOSSARY_BYTES), 'csv')).toThrow('4 MiB');
    const duplicate =
      'a,' + 'b'.repeat(MAX_GLOSSARY_BYTES / 2) + '\na,' + 'c'.repeat(MAX_GLOSSARY_BYTES / 2);
    expect(() => parseGlossary(duplicate, 'csv')).toThrow('4 MiB');
    expect(() => parseGlossary(' '.repeat(MAX_GLOSSARY_FILE_BYTES + 1), 'csv')).toThrow('encoded');
    expect(() => parseGlossary(','.repeat(MAX_GLOSSARY_FILE_BYTES), 'csv')).toThrow('two columns');
    const imported = parseGlossary('\uFEFFsource,target\r\n a , b \r\n a , c \r\n', 'csv');
    expect(imported).toEqual([
      { source: 'a', target: 'b' },
      { source: 'a', target: 'c' },
    ]);
    expect(
      normalizeGlossary({
        ...emptyGlossary(),
        entries: mergeTerms([], imported, false).entries,
        automaticSources: ['a', 'missing'],
      }).automaticSources,
    ).toEqual(['a']);
  });
  it('merges duplicates deterministically and defaults to retaining existing terms', () => {
    const old = [{ source: 'a', target: 'A' }],
      incoming = [
        { source: 'a', target: 'A' },
        { source: 'a', target: 'B' },
        { source: 'b', target: 'C' },
      ];
    expect(mergeTerms(old, incoming, false)).toEqual({
      entries: [...old, { source: 'b', target: 'C' }],
      added: 1,
      identical: 1,
      conflicts: 1,
    });
    expect(mergeTerms(old, incoming, true).entries[0].target).toBe('B');
    expect(old[0].target).toBe('A');
  });
  it('ignores application and override glossary terms while preserving captured snapshots', () => {
    const settings = settingsDefaults();
    settings.translation.glossary = [{ source: 'wrong', target: 'wrong' }];
    const book: Book = {
      id: 'b',
      path: 'b',
      metadata: metadata(),
      chapters: [],
      omittedPageIds: [],
      cover: null,
      revision: 0,
      overrides: { translation: structuredClone(settings.translation), reader: null },
      glossary: emptyGlossary(),
    };
    book.glossary.enabled = true;
    book.glossary.entries.push({ source: '本', target: '书' });
    const captured = effectiveSettings(book, settings);
    book.glossary.entries[0].target = '书本';
    book.overrides.translation = null;
    expect(captured.glossary[0].target).toBe('书');
    expect(effectiveSettings(book, settings).glossary[0].target).toBe('书本');
  });
});
