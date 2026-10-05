import { t, uiJoin, type MessageKey, type UiText } from './i18n';
import type {
  TranslationBatchMode,
  TranslationBatchPreview,
  TranslationBatchResult,
} from './types';

export function batchCounts(preview: TranslationBatchPreview | null, mode: TranslationBatchMode) {
  let eligible = 0,
    omitted = 0,
    skipped = 0,
    busy = 0,
    replacing = 0;
  for (const page of preview?.pages || []) {
    if (page.omitted) omitted++;
    else if (mode === 'skip_translated' && page.translated) skipped++;
    else if (page.busy) busy++;
    else {
      eligible++;
      if (mode === 'replace' && page.hasWork) replacing++;
    }
  }
  return { total: preview?.pages.length ?? 0, eligible, omitted, skipped, busy, replacing };
}

export function batchFeedback(result: TranslationBatchResult): UiText {
  const message = (key: MessageKey, count: number): UiText => ({
    key,
    args: { count },
    fallback: t(key, 'en', { count }),
  });
  return uiJoin(
    [
      message(result.held ? 'batch.jobs_held' : 'batch.jobs_added', result.jobs.length),
      ...(result.omitted ? [message('batch.result_omitted', result.omitted)] : []),
      ...(result.skipped ? [message('batch.result_skipped', result.skipped)] : []),
      ...(result.busy ? [message('batch.result_busy', result.busy)] : []),
      ...(result.changed ? [message('batch.result_changed', result.changed)] : []),
    ],
    ' · ',
  );
}
