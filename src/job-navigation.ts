import type { Book, Project } from './types';

/** Resolve current membership, never a job's potentially outdated displayed position. */
export async function jobDestination(
  path: string,
  pageId: string,
  openBook: (path: string) => Promise<Book>,
  openChapter: (path: string, chapterId: string) => Promise<Project>,
  current: () => boolean,
) {
  const book = await openBook(path);
  if (!current()) return;
  const chapter = book.chapters.find((c) => c.pageIds.includes(pageId));
  if (!chapter) throw new Error('This page is no longer available in the book.');
  const project = await openChapter(book.path, chapter.id);
  if (!current()) return;
  if (!project.pages.some((page) => page.id === pageId))
    throw new Error('This page is no longer available in the book.');
  return { book, chapterId: chapter.id, project, pageId };
}
