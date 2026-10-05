import { describe, it, expect } from 'vitest';
import { modelDirectory } from './model-storage';
describe('library-owned model directory', () => {
  it('derives one models folder without requiring another preference', () => {
    for (const [libraryDirectory, expected] of [
      ['', ''],
      ['E:\\漫画 library\\', 'E:\\漫画 library\\models'],
      ['E:\\', 'E:\\models'],
      ['\\\\server\\書籍\\', '\\\\server\\書籍\\models'],
      ['/home/books/', '/home/books/models'],
      ['/', '/models'],
    ])
      expect(modelDirectory({ libraryDirectory })).toBe(expected);
  });
});
