import { describe, it, expect, vi } from 'vitest';
import { jobDestination } from './job-navigation';
import type { Book, Project } from './types';

const book = {
  path: 'book:a',
  chapters: [{ id: 'new-chapter', pageIds: ['before', 'target'] }],
} as Book;
const project = { pages: [{ id: 'before' }, { id: 'target' }] } as Project;
describe('job page destination', () => {
  it('resolves the current chapter by ID rather than the old job location', async () => {
    const chapter = vi.fn().mockResolvedValue(project);
    expect(
      await jobDestination(
        'book:a',
        'target',
        async () => book,
        chapter,
        () => true,
      ),
    ).toEqual({ book, chapterId: 'new-chapter', project, pageId: 'target' });
    expect(chapter).toHaveBeenCalledWith('book:a', 'new-chapter');
  });
  it('rejects deleted pages before loading any chapter', async () => {
    const chapter = vi.fn();
    await expect(
      jobDestination(
        'book:a',
        'gone',
        async () => book,
        chapter,
        () => true,
      ),
    ).rejects.toThrow('no longer available');
    expect(chapter).not.toHaveBeenCalled();
  });
  it('rejects a page removed while its chapter loads', async () => {
    await expect(
      jobDestination(
        'book:a',
        'target',
        async () => book,
        async () => ({ pages: [] }) as unknown as Project,
        () => true,
      ),
    ).rejects.toThrow('no longer available');
  });
  it('abandons superseded book and chapter reads', async () => {
    const chapter = vi.fn();
    expect(
      await jobDestination(
        'book:a',
        'target',
        async () => book,
        chapter,
        () => false,
      ),
    ).toBeUndefined();
    expect(chapter).not.toHaveBeenCalled();
    let current = true;
    expect(
      await jobDestination(
        'book:a',
        'target',
        async () => book,
        async () => {
          current = false;
          return project;
        },
        () => current,
      ),
    ).toBeUndefined();
  });
});
