import { emptyGlossary } from './glossary';
import { describe, it, expect } from 'vitest';
import {
  settingsDefaults,
  effectiveSettings,
  mergePreview,
  naturalCompare,
  metadata,
} from './book-state';
import type { Book, Page } from './types';
describe('book defaults and draft boundaries', () => {
  it('defaults to LaMa with glossary off while preserving saved choices and book ownership', () => {
    const s = settingsDefaults();
    expect(s).not.toHaveProperty('newBookAutoGlossary');
    expect(s.translation.glossaryEnabled).toBe(false);
    expect(s.translation.autoGlossary).toBe(false);
    expect(s.translation.cleanup.method).toBe('manga_lama');
    expect(emptyGlossary()).toMatchObject({ enabled: false, autoDetect: false });
    s.translation.cleanup.method = 'manga_aot';
    const saved = JSON.parse(JSON.stringify(s));
    expect(saved.translation.cleanup.method).toBe('manga_aot');
    expect(saved).not.toHaveProperty('newBookAutoGlossary');
    const book = {
      metadata: metadata(),
      overrides: { translation: null, reader: null },
      glossary: {
        ...emptyGlossary(),
        enabled: true,
        autoDetect: true,
        entries: [{ source: 'アリス', target: '爱丽丝' }],
      },
    } as Book;
    expect(effectiveSettings(book, s)).toMatchObject({
      glossaryEnabled: true,
      autoGlossary: true,
      glossary: book.glossary.entries,
    });
    expect(emptyGlossary().entries).toEqual([]);
    book.glossary.enabled = false;
    expect(effectiveSettings(book, s).glossary).toEqual([]);
    expect(book.glossary.entries).toEqual([{ source: 'アリス', target: '爱丽丝' }]);
  });
  it('inherits global defaults while keeping concurrency application-wide', () => {
    const s = settingsDefaults();
    expect(s.translation.device).toBe('directml');
    expect(s.translation.cleanup.device).toBe('directml');
    s.translation.contextPages = 2;
    s.concurrentBooks = 3;
    const b: Book = {
      id: 'b',
      glossary: emptyGlossary(),
      path: 'x',
      metadata: metadata(),
      chapters: [],
      omittedPageIds: [],
      cover: null,
      revision: 0,
      overrides: { translation: null, reader: null },
    };
    expect(effectiveSettings(b, s).contextPages).toBe(2);
    b.metadata.language = 'ko';
    b.overrides.translation = { ...s.translation, device: 'cpu', contextPages: 5 };
    expect(effectiveSettings(b, s)).toMatchObject({
      sourceLanguage: 'ko',
      contextPages: 5,
      autoGlossary: false,
      device: 'cpu',
      cleanup: { device: 'directml' },
    });
    expect(effectiveSettings(b, s)).not.toHaveProperty('concurrency');
    expect(s.concurrentBooks).toBe(3);
    expect(s.translation.sourceLanguage).toBe('ja');
    expect(s.translation.device).toBe('directml');
  });
  it('draft changes do not mutate applied settings or the other appearance profile', () => {
    const applied = settingsDefaults(),
      draft = structuredClone(applied);
    draft.appearance.night.filters.inversion = 1;
    draft.translation.contextPages = 6;
    draft.reader.layout = 'paged';
    expect(applied.translation.contextPages).toBe(0);
    expect(applied.reader.layout).toBe('continuous');
    expect(applied.appearance.night.filters.inversion).toBe(0);
    expect(draft.appearance.day.filters.inversion).toBe(0);
  });
  it('initializes current preferences and naturally sorts chapter labels', () => {
    expect(settingsDefaults()).toMatchObject({
      libraryView: 'grid',
      librarySort: 'title',
      reader: { layout: 'continuous', zoom: 70 },
    });
    expect(['Chapter 10', 'Chapter 2'].sort(naturalCompare)).toEqual(['Chapter 2', 'Chapter 10']);
  });
  it('deduplicates source membership across successive import scans without losing the draft', () => {
    const p = {
      id: '1',
      name: 'p.png',
      source: { path: 'a', kind: 'image', entry: null, index: 0 },
    } as Page;
    const result = mergePreview(
      {
        pages: [p],
        chapters: [{ id: 'c1', title: 'One', pageIds: ['1'], read: false }],
        omittedPageIds: [],
        warnings: [],
      },
      {
        pages: [{ ...p, id: '2' }],
        chapters: [{ id: 'c2', title: 'Two', pageIds: ['2'], read: false }],
        omittedPageIds: [],
        warnings: [],
      },
    );
    expect(result.pages).toHaveLength(1);
    expect(result.chapters[0].pageIds).toEqual(['1']);
    expect(result.chapters[1].pageIds).toEqual([]);
    expect(result.warnings[0]).toContain('Duplicate');
  });
  it('retains omission marks and excludes marks for duplicate incoming sources', () => {
    const page = {
      id: 'existing',
      source: { path: 'a', kind: 'image', entry: null, index: 0 },
    } as Page;
    const result = mergePreview(
      { pages: [page], chapters: [], omittedPageIds: ['existing'], warnings: [] },
      {
        pages: [
          { ...page, id: 'duplicate' },
          { ...page, id: 'new', source: { ...page.source, path: 'b' } },
        ],
        chapters: [],
        omittedPageIds: ['duplicate', 'new'],
        warnings: [],
      },
    );
    expect(result.omittedPageIds).toEqual(['existing', 'new']);
  });
});
