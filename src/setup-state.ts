import { randomUuid } from './browser-id';
import type { Settings, Readiness, Provider, ModelPack } from './types';
export const languages = [
  ['ja', 'Japanese'],
  ['zh-Hans', 'Simplified Chinese'],
  ['zh-Hant', 'Traditional Chinese'],
  ['en', 'English'],
  ['ko', 'Korean'],
  ['fr', 'French'],
  ['de', 'German'],
  ['es', 'Spanish'],
  ['ar', 'Arabic'],
  ['hi', 'Hindi'],
  ['ru', 'Russian'],
  ['th', 'Thai'],
  ['vi', 'Vietnamese'],
  ['ta', 'Tamil'],
].map(([value, label]) => ({ value, label }));
export const targetLanguages = languages.filter((l) => !['ru', 'th', 'vi', 'ta'].includes(l.value));
export const languageName = (value: string) =>
  languages.find((l) => l.value === value)?.label || value;
export function setupSteps(mode: string, method = 'manga_lama') {
  return [
    'library',
    'pipeline',
    'services',
    ...(mode === 'local' || method !== 'solid' ? ['models'] : []),
    'review',
  ];
}
export const stepNames: Record<string, string> = {
  library: 'Library and languages',
  pipeline: 'Pipeline',
  services: 'Translation service',
  models: 'Downloads',
  review: 'Review setup',
};
export function stepIssues(
  report: Readiness,
  step: string,
  mode: string,
  complete = false,
  method = 'manga_lama',
) {
  const index = setupSteps(mode, method).indexOf(step);
  return report.issues.filter(
    (i) =>
      complete ||
      step === 'review' ||
      (i.section === 'library' && index >= 1) ||
      (i.section === 'pipeline' && ((i.code === 'languages' && index >= 1) || index >= 2)) ||
      (i.code === 'pack:rtdetr_int8' && index >= 2) ||
      (i.section === 'services' && index >= 3),
  );
}
export function newProvider(): Provider {
  return {
    thinking: false,
    thinkingPolicy: null,
    id: randomUuid(),
    name: 'New service',
    service: 'llm',
    protocol: 'openai',
    endpoint: '',
    model: '',
    vision: false,
    region: '',
    appId: '',
    rateLimit: 1,
    simpleTranslation: true, instructions: { textTranslation: null, simpleTextTranslation: null, visionTranslation: null, glossaryDetection: null },
  };
}
export function testSignature(s: Settings, includeInstructions = true) {
  const t = s.translation;
  const provider = s.providers.find((p) => p.id === t.providerId);
  return JSON.stringify([
    t.sourceLanguage,
    t.targetLanguage,
    t.mode,
    t.ocr,
    t.glossaryEnabled,
    t.glossary,
    t.deeplGlossaryId,
    t.contextPages,
    provider && { ...provider, simpleTranslation: includeInstructions ? provider.simpleTranslation : undefined, instructions: includeInstructions ? provider.instructions : undefined },
  ]);
}
export function readinessSignature(s: Settings) {
  return JSON.stringify([
    testSignature(s, false),
    s.libraryDirectory,
    s.translation.cleanup,
    s.translation.device,
    s.concurrentBooks,
  ]);
}
export function requiredModelIds(s: Settings['translation'], models: ModelPack[]) {
  const ids = ['rtdetr_int8'];
  if (s.mode === 'local') {
    if (s.ocr === 'manga') ids.push('manga_ocr_onnx');
    else if (s.ocr === 'pp') {
      ids.push('pp_det');
      const pack = models.find(
        (p) => p.kind === 'recognizer' && p.languages.includes(s.sourceLanguage),
      );
      if (pack) ids.push(pack.id);
    }
  }
  if (s.cleanup.method !== 'solid') ids.push('inpaint_' + s.cleanup.method);
  return ids;
}
export function ollamaDestination(endpoint: string) {
  const address = endpoint.trim() || 'http://localhost:11434';
  try {
    const host = new URL(address).hostname;
    return `${host === 'localhost' || host === '[::1]' || /^127\.\d+\.\d+\.\d+$/.test(host) ? 'This computer' : 'Ollama server'} · ${address}`;
  } catch {
    return address;
  }
}
