import type { UiText } from './i18n';
import { instructionIssue } from './instructions';
import { credentialIdentity } from './credential-autosave';
import { modelDirectory } from './model-storage';
// Browser-only simulation. No credentials, checkpoints, or service requests leave this preview.
import { requiredModelIds, setupSteps, stepIssues } from './setup-state';
import modelCatalog from '../assets/models.json';
import type {
  Settings,
  Readiness,
  ModelPack,
  Provider,
  OllamaModels,
  OllamaStatus,
  ActionName,
  Availability,
  Page,
} from './types';
export function previewAction(
  s: Settings,
  action: ActionName,
  credentials: string[],
  verified: string[],
  page?: Page | null,
  base?: Page | null,
): Availability {
  if (action === 'region_translate') {
    const copy = structuredClone(s);
    copy.translation.mode = 'local';
    return previewAction(copy, 'service', credentials, verified);
  }
  if (action === 'region_read' && s.translation.mode === 'vision')
    return previewAction(s, 'service', credentials, verified);
  const report = previewReadiness(s, credentials, verified);
  let issues = report.issues.filter((i) => {
    if (action === 'region_read')
      return (
        i.section !== 'services' &&
        !i.code.startsWith('pack:inpaint_') &&
        i.code !== 'pack:rtdetr_int8'
      );
    if (action === 'service')
      return i.section === 'services' || ['languages', 'mode'].includes(i.code);
    if (action === 'translate') return true;
    if (action === 'prepare') return i.section !== 'services';
    return i.code.startsWith('pack:inpaint_');
  });
  if (
    ['translate', 'service'].includes(action) &&
    !s.translation.providerId &&
    !issues.some((i) => i.code === 'service')
  )
    issues.push({
      code: 'service',
      section: 'services',
      message: 'Select and configure a translation service in Settings → Translation → Services.',
    });
  if (action === 'prepare' && s.translation.mode !== 'local')
    issues.push({
      code: 'mode',
      section: 'pipeline',
      message: 'Choose Local OCR to prepare a page for manual translation.',
    });
  if (action === 'cleanup' && !page?.regions.some((r) => r.prepared || r.target.trim()))
    issues.push({
      code: 'regions',
      section: 'page',
      message: 'Prepare or translate a page before re-cleaning.',
    });
  if (action === 'edit' && page && base) {
    const identity = (p: Page) =>
      JSON.stringify([
        p.background,
        p.regions.map((r) => [
          r.id,
          r.bbox,
          r.bubble,
          r.allowFill,
          r.overlayOnly,
          r.style.fill,
          r.prepared,
          r.prepared || !!r.target.trim(),
        ]),
      ]);
    if (page.background || (page.cleanup && identity(page) === identity(base))) issues = [];
    if (
      !page.regions.some((r) => !r.overlayOnly && !r.allowFill && (r.prepared || r.target.trim()))
    )
      issues = [];
  }
  return { ready: !issues.length, issues };
}
export function previewOllamaModels(endpoint: string): OllamaModels {
  const connected = !endpoint.includes('offline');
  return {
    endpoint: endpoint || 'http://localhost:11434',
    connected,
    message: connected
      ? null
      : 'Cannot connect to Ollama. Start Ollama and check the server address and network access.',
    models: connected
      ? [
          { name: 'local-vision:4b', size: 3_300_000_000, digest: 'preview-vision', remote: false },
          { name: 'local-text:3b', size: 2_000_000_000, digest: 'preview-text', remote: false },
          { name: 'remote:cloud', size: 0, digest: 'preview-cloud', remote: true },
        ]
      : [],
  };
}
export function previewOllamaDetails(endpoint: string, model: string): OllamaStatus {
  const inventory = previewOllamaModels(endpoint);
  const installed = inventory.models.find((m) => m.name === model);
  return {
    endpoint: inventory.endpoint,
    connected: inventory.connected,
    message:
      inventory.message ||
      (!installed
        ? model
          ? `Model '${model}' is not installed on this Ollama server. Install it in Ollama, then Refresh.`
          : 'Select an installed Ollama model.'
        : null),
    model: installed
      ? {
          ...installed,
          capabilities: model.includes('vision') ? ['completion', 'vision'] : ['completion'],
          thinking: { state: 'switchable', on: 'low', off: 'none' },
        }
      : null,
  };
}
export const previewModels = modelCatalog as ModelPack[];
export const previewCatalog = {
  example: {
    name: 'Example service (preview)',
    api: 'https://example.invalid/v1',
    models: {
      'example-vision': { modalities: { input: ['image', 'text'] } },
      'example-text': { modalities: { input: ['text'] } },
    },
  },
};
export const scope = credentialIdentity;
export function previewReadiness(
  s: Settings,
  credentials: string[],
  verified: string[],
): Readiness {
  const p = s.providers.find((p) => p.id === s.translation.providerId),
    t = s.translation;
  const credential = !!p && credentials.includes(scope(p));
  const isOllama = p?.service === 'llm' && p.protocol === 'ollama';
  const ollama = isOllama ? previewOllamaDetails(p.endpoint, p.model) : null;
  const issues: Readiness['issues'] = [];
  const issue = (code: string, section: string, message: UiText) =>
    issues.push({ code, section, message });
  if (!Number.isInteger(s.concurrentBooks) || s.concurrentBooks < 1 || s.concurrentBooks > 4)
    issue('limits', 'services', 'Use 1–4 concurrent books.');
  if (!Number.isInteger(t.contextPages) || t.contextPages < 0 || t.contextPages > 20)
    issue('limits', 'pipeline', 'Use 0–20 context pages.');
  if (!s.libraryDirectory) issue('library', 'library', 'Choose a library folder.');
  if (t.mode === 'local' && t.ocr === 'manga' && t.sourceLanguage !== 'ja')
    issue(
      'ocr_language',
      'pipeline',
      'Manga OCR reads Japanese only. Choose PP-OCRv5 for this source language.',
    );
  if (!p && (t.mode === 'vision' || t.providerId))
    issue('service', 'services', 'Select and configure a translation service.');
  if (p && t.mode === 'vision' && (p.service !== 'llm' || (!isOllama && !p.vision)))
    issue(
      'vision',
      'services',
      'Vision requires an image-capable model. Select one or choose Local OCR.',
    );
  if (p?.service === 'llm' && !p.model) issue('model', 'services', 'Select or enter a model ID.');
  const promptIssue = instructionIssue(p);
  if (promptIssue) issue('instructions', 'services', promptIssue);
  if (p && !isOllama && !credential)
    issue('credential', 'services', 'Enter an API key for the selected service and endpoint.');
  if (p?.service === 'baidu' && !p.appId)
    issue('app_id', 'services', 'Enter the Baidu application ID.');
  if (ollama?.message) issue('ollama', 'services', ollama.message);
  else if (ollama?.model?.remote)
    issue(
      'ollama',
      'services',
      'Choose a model installed on this Ollama server. Cloud-backed models are not supported here.',
    );
  else if (ollama && t.mode === 'vision' && !ollama.model?.capabilities.includes('vision'))
    issue(
      'ollama',
      'services',
      'This Ollama model cannot read images. Choose a vision model or Local OCR.',
    );
  const ids = requiredModelIds(t, previewModels);
  const packs = previewModels.map((p) => {
    const required = ids.includes(p.id),
      included = p.distribution === 'bundled';
    const valid = included || verified.includes(`${modelDirectory(s)}:${p.id}`);
    if (required && !valid)
      issue(
        `pack:${p.id}`,
        'models',
        modelDirectory(s)
          ? `Download or verify ${p.name} in Local models.`
          : 'Choose a library folder before downloading model packs.',
      );
    return {
      id: p.id,
      name: p.name,
      distribution: p.distribution,
      required,
      status: valid ? 'verified' : 'missing',
      bytes: p.files.reduce((n, f) => n + f.bytes, 0),
      path: included ? 'App cache' : modelDirectory(s),
    };
  });
  const report: Readiness = {
    ready: !issues.length,
    advancement: {},
    issues,
    packs,
    credentialStored: credential,
    credentialRequired: !!p && !isOllama,
    ollama,
    destination: isOllama
      ? `${/localhost|127\.0\.0\.1|\[::1\]/.test(p.endpoint) ? 'This computer' : 'Ollama server'} · ${ollama!.endpoint}`
      : p?.service === 'llm'
        ? p.endpoint || 'https://api.openai.com/v1/'
        : p
          ? `Official ${p.service} API`
          : '',
  };
  const stages = setupSteps(t.mode, t.cleanup.method);
  for (const [index, step] of stages.entries()) {
    const blocked = stepIssues(
      report,
      stages[index + 1] || 'review',
      t.mode,
      step === 'review',
      t.cleanup.method,
    );
    report.advancement[step] = { ready: !blocked.length, issues: blocked };
  }
  return report;
}
