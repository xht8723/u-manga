import type { CapturedJob, Job } from './types';
// Preview-only opaque key. Native snapshots use SHA-256; neither exposes destinations.
function key(text: string) {
  let a = 2166136261, b = 5381;
  for (let i = 0; i < text.length; i++) { const c = text.charCodeAt(i); a = Math.imul(a ^ c, 16777619); b = Math.imul(b, 33) ^ c; }
  return `preview-${(a >>> 0).toString(16)}-${(b >>> 0).toString(16)}-${text.length}`;
}
/** Browser fixtures mirror the native IPC projection, keeping captured inputs private. */
export function jobView(job: CapturedJob): Job {
  const { settings: s, provider: p, modelDirectory, glossaryCheckpoint, ...view } = job;
  const { glossary: _glossary, ...requirements } = s;
  return {
    ...view,
    settings: { sourceLanguage:s.sourceLanguage,targetLanguage:s.targetLanguage,mode:s.mode,autoGlossary:s.autoGlossary,glossaryEnabled:s.glossaryEnabled,cleanup:s.cleanup },
    provider: { name:p.name,model:p.model,service:p.service },
    glossaryCheckpoint: glossaryCheckpoint ? { added: glossaryCheckpoint.added } : null,
    requirementsKey: key(JSON.stringify([job.kind, requirements, p, modelDirectory,job.fresh])),
  };
}
