import { describe, expect, it } from 'vitest';
import { batchCounts, batchFeedback } from './batch-translation';
import { tr } from './i18n';
import type { TranslationBatchPreview } from './types';
const preview: TranslationBatchPreview = {
  bookId: 'book',
  bookTitle: 'Original book name',
  chapterTitle: null,
  pages: [
    {
      id: 'partial',
      revision: 1,
      number: 0,
      translated: true,
      omitted: false,
      hasWork: true,
      busy: false,
    },
    {
      id: 'fresh',
      revision: 0,
      number: 1,
      translated: false,
      omitted: false,
      hasWork: false,
      busy: false,
    },
    {
      id: 'manual',
      revision: 2,
      number: 2,
      translated: false,
      omitted: false,
      hasWork: true,
      busy: false,
    },
    {
      id: 'busy',
      revision: 1,
      number: 3,
      translated: false,
      omitted: false,
      hasWork: true,
      busy: true,
    },
    {
      id: 'busy-translated',
      revision: 1,
      number: 4,
      translated: true,
      omitted: false,
      hasWork: true,
      busy: true,
    },
  ],
};
describe('batch translation choices', () => {
  it('counts omitted pages before translated or busy flags in both modes', () => {
    const marked = structuredClone(preview);
    marked.pages[0].omitted = true;
    marked.pages[1].omitted = true;
    marked.pages[4].omitted = true;
    expect(batchCounts(marked, 'skip_translated')).toMatchObject({
      eligible: 1,
      omitted: 3,
      skipped: 0,
      busy: 1,
    });
    expect(batchCounts(marked, 'replace')).toMatchObject({
      eligible: 1,
      omitted: 3,
      skipped: 0,
      busy: 1,
      replacing: 1,
    });
    for (const page of marked.pages) page.omitted = true;
    expect(batchCounts(marked, 'replace')).toMatchObject({ eligible: 0, omitted: 5, busy: 0 });
    const feedback = batchFeedback({
      jobs: [],
      warnings: [],
      held: false,
      omitted: 5,
      skipped: 0,
      busy: 0,
      changed: 0,
    });
    expect(tr(feedback, 'en')).toContain('5 omitted pages skipped');
    expect(tr(feedback, 'zh-Hans')).toContain('已排除 5 个标记为不翻译的页面');
  });
  it('matches native classification without double counting translated busy pages', () => {
    expect(batchCounts(preview, 'skip_translated')).toEqual({
      total: 5,
      eligible: 2,
      skipped: 2,
      omitted: 0,
      busy: 1,
      replacing: 0,
    });
    expect(batchCounts(preview, 'replace')).toEqual({
      total: 5,
      eligible: 3,
      skipped: 0,
      omitted: 0,
      busy: 2,
      replacing: 2,
    });
    expect(batchCounts(null, 'skip_translated').eligible).toBe(0);
  });
  it('retains structured outcomes that switch language without another submission', () => {
    const feedback = batchFeedback({
      jobs: [],
      warnings: [],
      held: true,
      skipped: 3,
      omitted: 0,
      busy: 1,
      changed: 2,
    });
    expect(tr(feedback, 'en')).toBe(
      '0 jobs added as paused · 3 translated pages kept · 1 busy pages skipped · 2 changed or removed pages skipped',
    );
    expect(tr(feedback, 'zh-Hans')).toContain('已保留 3 个有译文的页面');
    expect(tr(feedback, 'zh-Hans')).toContain('已跳过 2 个已更改或移除的页面');
  });
  it('leaves receipt warnings to their single grouped notification path', () => {
    const warnings = [{ id: 'job', message: 'Durable warning' }];
    const result = { jobs: [], held: false, skipped: 0, omitted: 0, busy: 0, changed: 0, warnings };
    expect(tr(batchFeedback(result))).toBe('0 jobs added');
    expect(result.warnings).toBe(warnings);
  });
});
