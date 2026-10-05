import { readFileSync } from 'node:fs';
import { expect, test } from 'vitest';
import { LibraryPublication, libraryPath } from './library-publication';
type Summary = { id: string; path: string; title: string };
const summary = (title: string, path = 'E:/old/book.umanga'): Summary => ({ id: 'book', path, title });
const fixture = () => {
  const owner = new LibraryPublication<Summary>();
  owner.activate('E:/old'); owner.acceptFull(owner.beginFull(), [summary('Original')]);
  return owner;
};
test('late partial summaries and failures cannot own newer metadata', () => {
  const owner = fixture();
  const old = owner.beginBook('book', summary('').path);
  const next = owner.beginBook('book', summary('').path);
  expect(owner.acceptBook(next, summary('New'))).toBe(true);
  expect(owner.acceptBook(old, summary('Old'))).toBe(false);
  expect(owner.current(old)).toBe(false);
  expect(owner.values()[0].title).toBe('New');
});
test('relocation invalidates old responses even when book IDs are unchanged', () => {
  const owner = fixture(), old = owner.beginBook('book', summary('').path);
  owner.activate('E:/new'); owner.acceptFull(owner.beginFull(), [summary('New', 'E:/new/book.umanga')]);
  expect(owner.acceptBook(old, summary('Old'))).toBe(false);
  expect(owner.values()[0].path).toBe('E:/new/book.umanga');
});
test('full lists preserve newer partial entries and do not resurrect deletions', () => {
  const owner = fixture(), old = owner.beginFull();
  owner.acceptBook(owner.beginBook('book', summary('').path), summary('New'));
  expect(owner.acceptFull(old, [summary('Old')])).toBe(true);
  expect(owner.values()[0].title).toBe('New');
  const beforeDeletion = owner.beginFull(); owner.remove('book');
  owner.acceptFull(beforeDeletion, [summary('Old')]);
  owner.acceptFull(owner.beginFull(), [summary('Stale index')]);
  expect(owner.values()).toEqual([]);
  owner.created('book'); owner.acceptFull(owner.beginFull(), [summary('Reimported')]);
  expect(owner.values()[0].title).toBe('Reimported');
});
test('mutation intents supersede reads, and different paths cannot impersonate a book', () => {
  const owner = fixture(), old = owner.beginBook('book', summary('').path);
  owner.changed('book'); expect(owner.acceptBook(old, summary('Old'))).toBe(false);
  expect(owner.acceptBook(owner.beginBook('book', summary('').path), summary('Wrong', 'E:/other/book.umanga'))).toBe(false);
  const a = owner.beginFull(), b = owner.beginFull();
  expect(owner.acceptFull(b, [summary('New')])).toBe(true);
  expect(owner.acceptFull(a, [summary('Old')])).toBe(false);
});
test('Windows aliases are stable without lowercasing non-Windows paths', () => {
  expect(libraryPath('\\\\?\\C:\\Library\\')).toBe(libraryPath('c:/library'));
  expect(libraryPath('\\\\?\\UNC\\Server\\Share\\Book')).toBe(libraryPath('\\\\server\\share\\book'));
  expect(libraryPath('/Books/Title')).not.toBe(libraryPath('/books/title'));
});
test('a full deletion suppresses older partial reads without blocking a new import', () => {
  const owner = fixture(), old = owner.beginBook('book', summary('').path);
  owner.acceptFull(owner.beginFull(), []);
  expect(owner.acceptBook(old, summary('Stale'))).toBe(false);
  owner.created('book'); owner.acceptFull(owner.beginFull(), [summary('Reimported')]);
  expect(owner.values()[0].title).toBe('Reimported');
});
test('production full and partial updates both use publication ownership', () => {
  const source = readFileSync(new URL('./AppShell.svelte', import.meta.url), 'utf8');
  expect(source).toContain('libraryPublication.acceptBook(');
  expect(source).toContain('libraryPublication.acceptFull(');
  expect(source).not.toContain('books[i] = summary');
});
