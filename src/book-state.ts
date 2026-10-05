import { randomUuid } from './browser-id';
import type { Settings, Book, Chapter, ImportPreview, Metadata } from './types';
import { defaults } from './appearance';
export const metadata = (): Metadata => ({
  title: '',
  creator: '',
  description: '',
  language: '',
  tags: [],
});
export const emptyPreview = (): ImportPreview => ({
  chapters: [],
  omittedPageIds: [],
  pages: [],
  warnings: [],
});
export const settingsDefaults = (): Settings => ({
  formatVersion: 15,
  concurrentBooks: 2,
  uiLanguage: 'system',
  setup: { completed: false, step: 'library' },
  appearance: defaults(),
  providers: [],
  libraryDirectory: '',
  libraryView: 'grid',
  librarySort: 'title',
  reader: { layout: 'continuous', zoom: 70 },
  translation: {
    sourceLanguage: 'ja',
    targetLanguage: 'zh-Hans',
    mode: 'local',
    ocr: 'manga',
    device: 'directml',
    providerId: '',
    glossaryEnabled: false,
    contextPages: 0,
    glossary: [],
    deeplGlossaryId: '',
    autoGlossary: false,
    cleanup: { method: 'manga_lama', strategy: 'automatic', device: 'directml' },
  },
});
export const chapter = (title = 'New chapter'): Chapter => ({
  id: randomUuid(),
  title,
  pageIds: [],
  read: false,
});
export function mergePreview(current: ImportPreview, incoming: ImportPreview): ImportPreview {
  const key = (p: ImportPreview['pages'][number]) => JSON.stringify(p.source);
  const seen = new Set(current.pages.map(key));
  const warnings = [...current.warnings, ...incoming.warnings];
  const pages = incoming.pages.filter((p) => {
    if (seen.has(key(p))) {
      warnings.push(`Duplicate skipped: ${p.name}`);
      return false;
    }
    seen.add(key(p));
    return true;
  });
  const ids = new Set(pages.map((p) => p.id));
  return {
    pages: [...current.pages, ...pages],
    omittedPageIds: [
      ...current.omittedPageIds,
      ...incoming.omittedPageIds.filter((id) => ids.has(id)),
    ],
    chapters: [
      ...current.chapters,
      ...incoming.chapters.map((c) => ({ ...c, pageIds: c.pageIds.filter((id) => ids.has(id)) })),
    ],
    warnings,
  };
}
export function effectiveSettings(book: Book, settings: Settings) {
  const s = structuredClone(book.overrides.translation || settings.translation);
  if (book.metadata.language) s.sourceLanguage = book.metadata.language;
  s.autoGlossary = book.glossary.autoDetect;
  s.glossaryEnabled = book.glossary.enabled;
  s.glossary = s.glossaryEnabled ? structuredClone(book.glossary.entries) : [];
  s.deeplGlossaryId = book.glossary.deeplGlossaryId;
  return s;
}
export const naturalCompare = (a: string, b: string) =>
  a.localeCompare(b, undefined, { numeric: true, sensitivity: 'base' });
