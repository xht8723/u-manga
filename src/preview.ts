import { randomUuid } from './browser-id';
import {
  previewDefaults,
  previewInstructions,
  validateInstructions,
  captureInstructions,
} from './instructions';
import { mergePreferences } from './preferences';
import { emptyGlossary, normalizeGlossary, parseGlossary, encodeGlossary } from './glossary';
import { mergePageEdits } from './editor-state';
import { modelDirectory } from './model-storage';
import type {
  Book,
  BookSummary,
  Page,
  ImportPreview,
  Settings,
  Provider,
  CapturedJob as Job,
  JobsBatch,
  ActionName,
  TranslationBatchPreview,
} from './types';
import { jobView } from './job-view';
import { metadata, chapter, settingsDefaults, effectiveSettings } from './book-state';
import {
  previewModels,
  previewCatalog,
  previewReadiness,
  previewAction,
  previewOllamaModels,
  previewOllamaDetails,
  scope,
} from './setup-preview';
import { stepIssues, setupSteps } from './setup-state';
import { hostedThinking } from './thinking';
const fixture = new URLSearchParams(location.search).get('fixture') || '';
const prefsKey = 'umanga-preferences-v15-' + fixture;
const credentials = JSON.parse(localStorage.getItem(prefsKey + '-credentials') || '[]') as string[];
const verified = JSON.parse(localStorage.getItem(prefsKey + '-models') || '[]') as string[];
const downloading = new Map<string, { cancelled: boolean }>();
const key =
  'umanga-library-preview-v19' +
  (['reader-cleanup', 'editor-progress', 'manual-progress', 'navigation'].includes(fixture)
    ? `-${fixture}`
    : '');
const controlFixture = new URLSearchParams(location.search).get('fixture') === 'controls';
const regionRequests = new Map<string, { cancelled: boolean; path: string; pageId: string }>();
const jobsFixture =
  new URLSearchParams(location.search).get('fixture') === 'jobs' || fixture === 'jobs-large';
const controlCatalog = controlFixture
  ? {
      preview: {
        name: 'Preview provider',
        api: 'https://example.invalid/v1',
        models: Object.fromEntries(
          [
            'deepseek-flash',
            'deepseek-v4-flash',
            'deepseek-v4-flash-vision-exp',
            'deepseek-v4-pro',
            ...Array.from(
              { length: 1500 },
              (_, i) => `fixture-model-${String(i).padStart(4, '0')}`,
            ),
          ].map((id) => [id, { modalities: { input: ['text', 'image'] } }]),
        ),
      },
    }
  : {};
type State = { books: Book[]; pages: Record<string, Page> };
const titles = [
  'Moonlit Station',
  'The Last Orchard',
  'Paper Kingdom',
  'After the Rain',
  'Northbound',
  'The Silent Sea',
  'A Summer in Ink',
  'Night Market',
  'Between Two Worlds',
  'Golden Hour',
  'The Glass Garden',
  'Letters from Kyoto',
  'Blue Meridian',
  'Small Constellations',
  'The Long Way Home',
  'Beyond the Clouds',
];
function page(name: string): Page {
  return {
    id: randomUuid(),
    number: 0,
    name,
    source: { path: `preview://${randomUuid()}`, kind: 'image', entry: null, index: 0 },
    width: 800,
    height: 1150,
    fingerprint: '',
    revision: 0,
    regions: [],
    rendered: null,
    background: null,
    cleanup: null,
    status: 'new',
    error: null,
  };
}
function seed(): State {
  const state: State = { books: [], pages: {} };
  const requested = Number(new URLSearchParams(location.search).get('fixture'));
  const largeChapters = new URLSearchParams(location.search).get('fixture') === 'chapters';
  const count = largeChapters || fixture === 'navigation' ? 1 : requested === 1000 ? 1000 : 16;
  for (let i = 0; i < count; i++) {
    const id = randomUuid();
    const chapters = Array.from({ length: 3 + (i % 5) }, (_, n) => {
      const c = chapter(`Chapter ${n + 1}`);
      c.read = n < i % 3;
      const pageCount = largeChapters ? 229 : fixture === 'navigation' ? 120 : jobsFixture ? 24 : 3;
      for (let j = 0; j < pageCount; j++) {
        const p = page(`${n + 1}-${j + 1}.png`);
        if (fixture === 'reader-cleanup' && j === 0) {
          p.regions = [
            {
              id: 'cleanup-review',
              bbox: [240, 180, 550, 700],
              bubble: null,
              kind: 'text',
              score: 1,
              source: 'こんにちは',
              target: '你好。',
              direction: 'vertical',
              style: {
                font: null,
                size: null,
                color: '#202020',
                fill: '#ffffff',
                lineGap: 0.2,
                outlineEnabled: false,
                outlineWidthPercent: 8,
                outlineColor: '#ffffff',
              },
              allowFill: false,
              overlayOnly: false,
              prepared: false,
              review:
                'Translation ready; cleanup needs review: no closed balloon interior found; adjust the region or import a cleaned background.',
            },
          ];
          p.status = 'review';
        }
        p.number = n * pageCount + j;
        state.pages[p.id] = p;
        c.pageIds.push(p.id);
      }
      return c;
    });
    state.books.push({
      id,
      path: `preview://${id}`,
      metadata: {
        ...metadata(),
        title: titles[i % titles.length] + (i >= 16 ? ` ${i + 1}` : ''),
        creator: ['M. Aoki', 'Y. Mori', 'R. Tanaka', 'S. Kato'][i % 4],
        language: 'ja',
        tags: [['Drama', 'Fantasy', 'Adventure', 'Slice of life'][i % 4]],
        description: '',
      },
      chapters,
      omittedPageIds: [],
      cover: null,
      overrides: { translation: null, reader: null },
      glossary: emptyGlossary(),
      revision: 0,
    });
  }
  return state;
}
let state: State = JSON.parse(localStorage.getItem(key) || 'null') || seed();
// Isolated first-run fixture; selecting a library applies for this preview session.
let unsetLibrary = new URLSearchParams(location.search).get('fixture') === 'unset';
if (new URLSearchParams(location.search).get('fixture') === '1000' && state.books.length !== 1000)
  state = seed();
if (new URLSearchParams(location.search).get('fixture') === 'chapters') state = seed();
if (jobsFixture) state = seed();
const previewJobs: Job[] = jobsFixture
  ? Array.from({ length: fixture === 'jobs-large' ? 10000 : 126 }, (_, i) => {
      const book = state.books[i % 4];
      const chapter = book.chapters[Math.floor(i / 96) % book.chapters.length];
      const number = Math.floor(i / 4) % chapter.pageIds.length;
      const status: Job['status'] =
        i < 8
          ? 'running'
          : i < 55
            ? 'queued'
            : i < 64
              ? 'paused'
              : i < 118
                ? 'complete'
                : i < 122
                  ? 'failed'
                  : 'cancelled';
      const mode = i % 2 ? 'local' : 'vision';
      const stage =
        status === 'complete'
          ? 'done'
          : status === 'queued'
            ? 'waiting'
            : status === 'failed'
              ? 'translating'
              : [
                  'loading',
                  'detecting',
                  mode === 'local' ? 'ocr' : 'translating',
                  'translating',
                  'rendering',
                  'saving',
                ][i % 6];
      return {
        id: `jobs-fixture-${i}`,
        kind: i % 3 === 0 ? 'cleanup' : 'translation',
        modelDirectory: 'Browser preview/models',
        sequence: i + 1,
        stopping: false,
        project: book.path,
        pageId: chapter.pageIds[number],
        status,
        stage,
        steps: [],
        pageRevision: null,
        stageDetail: stage === 'translating' ? 'Batch 2 / 4 · 6 regions' : '',
        error:
          status === 'failed'
            ? 'Provider rate limit reached after retries. Cached translations are saved; retry when the provider is available.'
            : null,
        created: new Date(Date.UTC(2026, 8, 24, 10, 0, i)).toISOString(),
        elapsedMs: status === 'complete' ? 18400 + i * 350 : 0,
        device: mode === 'local' ? 'DirectML · output validated against CPU' : '',
        settings: { ...settingsDefaults().translation, mode },
        provider: {
          id: 'preview',
          name: 'Preview provider',
          service: 'llm',
          protocol: 'openai',
          endpoint: 'https://example.invalid',
          model: 'Sample model',
          vision: true,
          region: '',
          appId: '',
          rateLimit: 1,
          simpleTranslation: true,
          instructions: {
            textTranslation: null,
            simpleTextTranslation: null,
            visionTranslation: null,
            glossaryDetection: null,
          },
          thinking: false,
        },
        location: {
          bookId: book.id,
          bookTitle: book.metadata.title,
          chapterId: chapter.id,
          chapterTitle: chapter.title,
          pageNumber: number + 1,
          chapterPages: chapter.pageIds.length,
        },
      };
    })
  : [];
let jobsHeld = false,
  jobsVersion = 0,
  jobSequence = 10000;
const clocks = new Map<string, { start: number; base: number }>();
function timing(job: Job, running: boolean) {
  const old = clocks.get(job.id);
  if (old) job.elapsedMs = old.base + performance.now() - old.start;
  if (running && !old) clocks.set(job.id, { start: performance.now(), base: job.elapsedMs });
  if (!running) clocks.delete(job.id);
  job.timerRunning = running;
}
function jobsBatch(changed = previewJobs): JobsBatch {
  for (const job of changed) timing(job, job.status === 'running' || !!job.stopping);
  return structuredClone({
    jobs: changed.map(jobView),
    removed: [],
    held: jobsHeld,
    recovering: false,
    errors: [],
    generation: 1,
    version: ++jobsVersion,
  });
}
function publishJobs(changed = previewJobs) {
  const batch = jobsBatch(changed);
  window.dispatchEvent(new CustomEvent('umanga-jobs', { detail: batch }));
  return batch;
}
function updateJob(job: Job, action: string) {
  if (['resume', 'retry'].includes(action) && jobsHeld) throw Error('Use Start all to resume jobs');
  if (job.status === 'complete') return;
  job.stopping = action === 'pause' && job.status === 'running';
  job.status = action === 'pause' ? 'paused' : action === 'cancel' ? 'cancelled' : 'queued';
  job.sequence = ++jobSequence;
  if (job.status === 'queued') {
    job.stage = 'waiting';
    job.stageDetail = '';
    job.error = null;
  }
  if (job.stopping)
    setTimeout(() => {
      job.stopping = false;
      timing(job, false);
      job.sequence = ++jobSequence;
      publishJobs([job]);
    }, 650);
}
function persist() {
  // Large performance fixtures stay in memory; normal preview books remain persistent.
  if (
    ['1000', 'chapters', 'jobs', 'jobs-large'].includes(
      new URLSearchParams(location.search).get('fixture') || '',
    )
  )
    return;
  localStorage.setItem(key, JSON.stringify(state));
}
function summaries(): BookSummary[] {
  return state.books.map((b) => ({
    path: b.path,
    id: b.id,
    metadata: b.metadata,
    chapters: b.chapters.length,
    completed: b.chapters.filter((c) => c.read).length,
    pages: b.chapters.reduce((n, c) => n + c.pageIds.length, 0),
    translated: 0,
    review: 0,
    cover: b.cover,
    coverPage: b.chapters.find((c) => c.pageIds.length)?.pageIds[0] || null,
    updated: '2026-09-23',
  }));
}
function artwork(id: string) {
  const n = state.books.findIndex((b) => b.chapters.some((c) => c.pageIds.includes(id)));
  const palette = [
    ['#172936', '#ddd1b7', '#ef9678'],
    ['#3b4d48', '#e1c7a4', '#bf7764'],
    ['#292a44', '#cbc6db', '#bb9d65'],
    ['#e3e0cd', '#3d5c55', '#b06553'],
  ][Math.max(0, n) % 4];
  const b = state.books[Math.max(0, n)];
  const p = state.pages[id];
  const label = (b?.metadata.title || 'Preview page').replace(/[<>&]/g, '');
  return (
    'data:image/svg+xml;charset=utf-8,' +
    encodeURIComponent(
      `<svg xmlns="http://www.w3.org/2000/svg" width="800" height="1150" viewBox="0 0 800 1150"><rect width="800" height="1150" fill="${palette[0]}"/><circle cx="570" cy="345" r="210" fill="${palette[1]}" opacity=".88"/><path d="M0 660L240 280 540 740 800 440V1150H0Z" fill="${palette[2]}"/><path d="M0 790L340 600 600 1000 800 740V1150H0Z" fill="${palette[0]}" opacity=".65"/><path d="M100 620H700M400 150V850" stroke="${palette[1]}" stroke-width="2" opacity=".45"/><text x="62" y="98" font-family="sans-serif" font-size="22" letter-spacing="7" fill="${palette[1]}">U—MANGA · PREVIEW</text><text x="62" y="978" font-family="Georgia" font-size="48" fill="${palette[1]}">${label}</text><text x="62" y="1040" font-family="sans-serif" font-size="20" fill="${palette[1]}">${p?.name || ''}</text></svg>`,
    )
  );
}
function makePreviewJob(
  book: Book,
  id: string,
  s: Settings,
  fresh: boolean,
  revision: number | null,
  kind: Job['kind'],
): Job {
  const chapter = book.chapters.find((c) => c.pageIds.includes(id))!;
  const provider = s.providers.find((p) => p.id === s.translation.providerId) || {
    id: 'preview',
    name: 'Preview service',
    service: 'llm',
    protocol: 'openai',
    endpoint: 'https://example.invalid',
    model: 'Preview model',
    vision: true,
    region: '',
    appId: '',
    rateLimit: 1,
    simpleTranslation: true,
    instructions: {
      textTranslation: null,
      simpleTextTranslation: null,
      visionTranslation: null,
      glossaryDetection: null,
    },
    thinking: false,
  };
  return {
    fresh,
    id: randomUuid(),
    kind,
    modelDirectory: modelDirectory(s),
    project: book.path,
    pageId: id,
    status: jobsHeld ? 'paused' : 'queued',
    stage: 'waiting',
    steps: [],
    pageRevision: revision,
    stageDetail: jobsHeld ? 'Jobs stopped' : '',
    error: null,
    created: new Date().toISOString(),
    elapsedMs: 0,
    settings: structuredClone(s.translation),
    provider: captureInstructions(provider, s.translation.sourceLanguage),
    sequence: ++jobSequence,
    stopping: false,
    location: {
      bookId: book.id,
      bookTitle: book.metadata.title,
      chapterId: chapter.id,
      chapterTitle: chapter.title,
      pageNumber: chapter.pageIds.indexOf(id) + 1,
      chapterPages: chapter.pageIds.length,
    },
  };
}
function batchSnapshot(book: Book, chapterId: string | null): TranslationBatchPreview {
  const chapter = chapterId ? book.chapters.find((c) => c.id === chapterId) : null;
  if (chapterId && !chapter) throw Error('Chapter no longer exists');
  const all = book.chapters.flatMap((c) => c.pageIds);
  const wanted = new Set(chapter ? chapter.pageIds : all);
  return {
    bookId: book.id,
    bookTitle: book.metadata.title,
    chapterTitle: chapter?.title ?? null,
    pages: all
      .filter((id) => wanted.has(id) && state.pages[id])
      .map((id) => {
        const p = state.pages[id];
        return {
          id,
          revision: p.revision,
          number: all.indexOf(id),
          translated: p.regions.some((r) => !!r.target.trim()),
          omitted: book.omittedPageIds.includes(id),
          hasWork: !!(p.regions.length || p.rendered || p.cleanup || p.background),
          busy:
            [...regionRequests.values()].some((r) => r.path === book.path && r.pageId === id) ||
            previewJobs.some(
              (j) =>
                j.project === book.path &&
                j.pageId === id &&
                (j.stopping || ['queued', 'running', 'paused'].includes(j.status)),
            ),
        };
      }),
  };
}
export async function previewCall(command: string, args: Record<string, any>): Promise<any> {
  const settings = (): Settings =>
    JSON.parse(localStorage.getItem(prefsKey) || 'null') || {
      ...settingsDefaults(),
      appearance: (() => {
        const a = settingsDefaults().appearance;
        if (new URLSearchParams(location.search).get('scale') === '1.6') {
          a.day.scale = 1.6;
          a.night.scale = 1.6;
        }
        return a;
      })(),
      ...(fixture && !fixture.startsWith('onboarding')
        ? { libraryDirectory: 'Browser preview', setup: { completed: true, step: 'review' } }
        : {}),
    };
  const find = () => {
    const b = state.books.find((b) => b.path === args.path);
    if (!b) throw Error('Book not found');
    return b;
  };
  const effective = (path?: string): Settings => {
    const s = settings(),
      b = state.books.find((b) => b.path === path);
    if (b) s.translation = effectiveSettings(b, s);
    return s;
  };
  const actionStatus = (action: ActionName, s: Settings, p?: Page, base?: Page) =>
    previewAction(s, action, credentials, verified, p, base);
  const jobStatus = (j: Job) => {
    const s = settings();
    s.translation = j.settings;
    s.providers = [j.provider];
    const result = actionStatus(
      j.kind === 'preparation' ? 'prepare' : j.kind === 'cleanup' ? 'cleanup' : 'translate',
      s,
      state.pages[j.pageId],
    );
    if (j.kind === 'preparation' && j.pageRevision !== state.pages[j.pageId]?.revision) {
      result.ready = false;
      result.issues.push({
        code: 'revision',
        section: 'page',
        message: 'Page changed; review it and confirm preparation again.',
      });
    }
    return result;
  };
  const requireReady = (status: ReturnType<typeof previewAction>) => {
    if (!status.ready) throw Error(status.issues.map((i) => i.message).join('\n'));
  };
  switch (command) {
    case 'prompt_defaults':
      return previewDefaults(args.sourceLanguage);
    case 'prompt_preview':
      return previewInstructions(args.profile, args.settings, args.task);
    case 'region_cancel': {
      const r = regionRequests.get(args.id);
      if (r) r.cancelled = true;
      return;
    }
    case 'region_read':
    case 'region_translate': {
      const r = args.request;
      if (jobsHeld) throw Error('Use Start all in Jobs first.');
      const page = state.pages[r.pageId];
      if (!page || page.revision !== r.expected)
        throw Error('Page changed; retry the region action');
      if (
        [...regionRequests.values()].some((v) => v.pageId === r.pageId) ||
        previewJobs.some(
          (j) => j.pageId === r.pageId && ['running', 'queued', 'paused'].includes(j.status),
        )
      )
        throw Error('This page already has an operation; wait for it to finish');
      if (
        [...regionRequests.values()].some((v) => v.path === r.path) ||
        previewJobs.some((j) => j.project === r.path && (j.status === 'running' || j.stopping))
      )
        throw Error('This book is processing another page; wait for it to finish');
      if (
        new Set(
          [...regionRequests.values()]
            .map((v) => v.path)
            .concat(
              previewJobs.filter((j) => j.status === 'running' || j.stopping).map((j) => j.project),
            ),
        ).size >= settings().concurrentBooks
      )
        throw Error('Processing limit reached; wait for an active book to finish');
      const status = actionStatus(
        command === 'region_read' ? 'region_read' : 'region_translate',
        effective(r.path),
      );
      requireReady(status);
      if (command === 'region_translate' && !r.region.source.trim())
        throw Error('Enter source text before translating this region');
      const operation = { cancelled: false, path: r.path, pageId: r.pageId };
      regionRequests.set(r.id, operation);
      const started = performance.now();
      try {
        window.dispatchEvent(
          new CustomEvent('region-progress', {
            detail: {
              id: r.id,
              stage: command === 'region_read' ? 'ocr' : 'translating',
              elapsedMs: 0,
            },
          }),
        );
        await new Promise((resolve) => setTimeout(resolve, 500));
        if (operation.cancelled || jobsHeld) throw Error('Cancelled');
        if (page.revision !== r.expected) throw Error('Page changed; region result discarded');
        return {
          id: r.id,
          pageId: r.pageId,
          regionId: r.region.id,
          expected: r.expected,
          text: command === 'region_read' ? '読み直した原文' : '重新翻译的文本',
          elapsedMs: performance.now() - started,
        };
      } finally {
        regionRequests.delete(r.id);
      }
    }
    case 'action_readiness': {
      const s = effective(args.path),
        b = state.books.find((b) => b.path === args.path);
      const actions = (args.actions || [
        'translate',
        'prepare',
        'cleanup',
        'edit',
        'service',
      ]) as ActionName[];
      const candidate = !actions.some((a) => a === 'cleanup' || a === 'edit')
        ? undefined
        : args.page ||
          b?.chapters
            .flatMap((c) => c.pageIds)
            .filter((id) => !args.pageIds || args.pageIds.includes(id))
            .map((id) => state.pages[id])
            .find((p) => p.regions.some((r) => r.prepared || r.target.trim()));
      return Object.fromEntries(
        actions.map((action) => {
          const applied = structuredClone(s);
          if (action === 'edit' && args.page?.cleanup)
            applied.translation.cleanup = args.page.cleanup.settings;
          return [action, actionStatus(action, applied, candidate, args.base)];
        }),
      );
    }
    case 'jobs_readiness':
      return Object.fromEntries(
        previewJobs.filter((j) => args.ids.includes(j.id)).map((j) => [j.id, jobStatus(j)]),
      );
    case 'editor_fixture_step': {
      if (!['editor-progress', 'manual-progress', 'navigation'].includes(fixture))
        throw Error('Editor fixture only');
      const job = previewJobs.find((j) => j.id === args.id)!;
      const p = state.pages[job.pageId];
      if (job.kind === 'preparation' && job.pageRevision !== p.revision)
        throw Error('Page changed; confirm preparation again.');
      if (args.stage === 'detecting' && !args.error) {
        if (job.fresh) {
          p.cleanup = null;
          p.rendered = null;
          p.background = null;
        }
        p.regions = [
          {
            id: 'detected-one',
            bbox: [120, 150, 500, 500],
            bubble: null,
            kind: 'text',
            score: 0.95,
            source: '',
            target: '',
            direction: 'auto',
            style: {
              font: null,
              size: null,
              color: '#000000',
              fill: '#ffffff',
              lineGap: 0.2,
              outlineEnabled: false,
              outlineWidthPercent: 8,
              outlineColor: '#ffffff',
            },
            allowFill: false,
            overlayOnly: false,
            prepared: job.kind === 'preparation',
            review: null,
          },
        ];
      }
      if (args.stage === 'ocr' && !args.error) p.regions[0].source = 'こんにちは';
      if (args.stage === 'translating' && !args.error) p.regions[0].target = '你好';
      if (args.stage === 'cleaning' && !args.error)
        p.cleanup = {
          settings: job.settings.cleanup,
          path: artwork(p.id),
          key: 'preview-cleanup',
          device: 'Preview',
          reviews: {},
        };
      if (args.stage === 'saving') {
        p.status = job.kind === 'preparation' ? 'prepared' : 'complete';
        p.rendered = artwork(p.id);
      }
      p.revision++;
      job.pageRevision = p.revision;
      job.stage = args.stage;
      job.status = args.error ? 'failed' : args.stage === 'saving' ? 'complete' : 'running';
      timing(job, job.status === 'running');
      job.error = args.error || null;
      const step = {
        stage: args.stage,
        status: args.error ? ('failed' as const) : ('complete' as const),
        detail: args.error ? 'Completed results were saved' : `${args.stage} saved`,
        error: args.error || null,
      };
      job.steps = [...job.steps.filter((s) => s.stage !== args.stage), step];
      job.sequence = ++jobSequence;
      persist();
      return publishJobs([job]);
    }
    case 'system_language':
      return navigator.language;
    case 'onboarding_language': {
      if (!['system', 'en', 'zh-Hans'].includes(args.language))
        throw Error('Invalid interface language');
      const s = settings();
      s.uiLanguage = args.language;
      localStorage.setItem(prefsKey, JSON.stringify(s));
      return;
    }
    case 'bootstrap':
      return {
        systemLocale: navigator.language,
        settings: unsetLibrary ? { ...settings(), libraryDirectory: '' } : settings(),
        catalog: controlFixture ? controlCatalog : previewCatalog,
        models: previewModels,
        jobs: structuredClone(previewJobs.map(jobView)),
        fonts: controlFixture
          ? [
              'Segoe UI',
              'Segoe UI Emoji',
              'Segoe UI Historic',
              'Segoe UI Symbol',
              'Segoe UI Variable',
              ...Array.from(
                { length: 1000 },
                (_, i) => `Fixture Font ${String(i).padStart(4, '0')}`,
              ),
            ]
          : ['Segoe UI', 'Arial'],
      };
    case 'preferences':
      validateInstructions(args.settings.providers);
      args.settings = mergePreferences(args.base || settings(), args.settings, settings());
      if (
        !Number.isInteger(args.settings.concurrentBooks) ||
        args.settings.concurrentBooks < 1 ||
        args.settings.concurrentBooks > 4
      )
        throw Error('Use 1–4 concurrent books.');
      if (args.settings.libraryDirectory !== settings().libraryDirectory && downloading.size)
        throw Error('Finish or cancel model downloads before changing the library.');
      unsetLibrary = !args.settings.libraryDirectory;
      localStorage.setItem(
        prefsKey,
        JSON.stringify({
          ...args.settings,
          setup: settings().setup,
        }),
      );
      return settings();
    case 'translation_readiness':
      if (fixture.includes('slow')) await new Promise((resolve) => setTimeout(resolve, 700));
      return previewReadiness(args.settings, credentials, verified);
    case 'ollama_models':
      return previewOllamaModels(args.endpoint);
    case 'thinking_capability':
      return args.profile.protocol === 'ollama'
        ? previewOllamaDetails(args.profile.endpoint, args.profile.model).model?.thinking || {
            state: 'managed',
            on: null,
            off: null,
          }
        : hostedThinking(args.profile);
    case 'ollama_model_details':
      return previewOllamaDetails(args.endpoint, args.model);
    case 'onboarding_save': {
      validateInstructions(args.settings.providers);
      args.settings = mergePreferences(args.base || settings(), args.settings, settings());
      if (settings().setup.completed) throw Error('Setup is already complete.');
      if (
        !setupSteps(
          args.settings.translation.mode,
          args.settings.translation.cleanup.method,
        ).includes(args.step)
      )
        throw Error('Unknown setup step');
      const next = { ...args.settings };
      const issues = stepIssues(
        previewReadiness(next, credentials, verified),
        args.step,
        next.translation.mode,
        args.complete,
        next.translation.cleanup.method,
      );
      const stages = setupSteps(next.translation.mode, next.translation.cleanup.method);
      const goingBack =
        !args.complete && stages.indexOf(args.step) < stages.indexOf(settings().setup.step);
      if (!goingBack && issues.length) throw Error(issues.map((i) => i.message).join('\n'));
      next.setup = { completed: args.complete, step: args.step };
      localStorage.setItem(prefsKey, JSON.stringify(next));
      return next;
    }
    case 'onboarding_appearance':
      localStorage.setItem(
        prefsKey,
        JSON.stringify({ ...settings(), appearance: args.appearance }),
      );
      return;
    case 'secret_set': {
      if (!args.value.trim()) throw Error('Enter a credential.');
      const id = scope(args.profile);
      if (!credentials.includes(id)) credentials.push(id);
      localStorage.setItem(prefsKey + '-credentials', JSON.stringify(credentials));
      return;
    }
    case 'secret_status':
      return credentials.includes(scope(args.profile));
    case 'service_test': {
      const report = previewReadiness(args.settings, credentials, verified);
      if (report.issues.some((i) => ['services', 'pipeline'].includes(i.section)))
        throw Error('Configure the selected service first.');
      const profile = args.settings.providers.find(
        (p: Provider) => p.id === args.settings.translation.providerId,
      );
      const simple =
        profile?.service === 'llm' &&
        profile.simpleTranslation &&
        args.settings.translation.mode !== 'vision';
      return {
        elapsedMs: 0,
        source: simple ? '123\n456' : '123',
        target: simple ? '123\n456' : '123',
      };
    }
    case 'model_storage':
      return { library: settings().libraryDirectory, directory: modelDirectory(settings()) };
    case 'model_downloads':
      return [...downloading.keys()];
    case 'model_verify':
      checkModelLibrary(settings(), args.library);
      return (
        args.id === 'rtdetr_int8' || verified.includes(modelDirectory(settings()) + ':' + args.id)
      );
    case 'model_cancel':
      if (downloading.has(args.id)) downloading.get(args.id)!.cancelled = true;
      return;
    case 'model_download': {
      if (args.id === 'rtdetr_int8') throw Error('Detector is included.');
      checkModelLibrary(settings(), args.library);
      const token = { cancelled: false };
      downloading.set(args.id, token);
      const root = modelDirectory(settings());
      await new Promise((resolve) => setTimeout(resolve, 1200));
      downloading.delete(args.id);
      if (token.cancelled) throw Error('Download cancelled; partial files retained.');
      verified.push(root + ':' + args.id);
      localStorage.setItem(prefsKey + '-models', JSON.stringify(verified));
      return;
    }
    case 'library_list':
      return unsetLibrary ? [] : structuredClone(summaries());
    case 'book_open':
      return structuredClone(find());
    case 'book_glossary_save': {
      const b = find();
      if (b.glossary.revision !== args.expected)
        throw Error(
          'Glossary changed elsewhere. Reopen it before saving; your draft is still available.',
        );
      const glossary = normalizeGlossary(args.glossary);
      glossary.automaticSources = glossary.automaticSources.filter(
        (source) =>
          b.glossary.automaticSources.includes(source) &&
          b.glossary.entries.some(
            (e) =>
              e.source === source &&
              glossary.entries.some((n) => n.source === source && n.target === e.target),
          ),
      );
      b.glossary = { ...glossary, revision: b.glossary.revision + 1 };
      b.revision++;
      persist();
      return structuredClone(b);
    }
    case 'glossary_parse':
      return parseGlossary(args.text, args.format);
    case 'glossary_export': {
      const text = encodeGlossary(args.entries, args.format);
      const url = URL.createObjectURL(new Blob([text], { type: 'text/plain;charset=utf-8' }));
      const a = document.createElement('a');
      a.href = url;
      a.download = args.destination;
      a.click();
      setTimeout(() => URL.revokeObjectURL(url), 1000);
      return;
    }
    case 'book_sources':
      return find().chapters.flatMap((c) =>
        c.pageIds.map((id) => ({ id, source: state.pages[id].source })),
      );
    case 'chapter_pages': {
      const b = find();
      const c = b.chapters.find((c) => c.id === args.chapterId)!;
      return structuredClone({
        path: b.path,
        id: b.id,
        title: c.title,
        settings: effectiveSettings(b, settings()),
        pages: c.pageIds.map((id) => state.pages[id]),
        omittedPageIds: b.omittedPageIds,
      });
    }
    case 'page_get':
      return structuredClone(state.pages[args.pageId]);
    case 'thumbnail':
    case 'page_image':
      return artwork(args.pageId);
    case 'source_thumbnail':
      return artwork(args.source.path);
    case 'jobs_list':
      return jobsBatch();
    case 'job_control': {
      const job = previewJobs.find((j) => j.id === args.id);
      if (!job) throw Error('Job not found');
      if (['complete', 'cancelled'].includes(job.status))
        throw Error('Completed or cancelled jobs cannot be changed');
      if (['resume', 'retry'].includes(args.action)) requireReady(jobStatus(job));
      updateJob(job, args.action);
      return { outcome: { changed: 1, skipped: 0, errors: [] }, state: publishJobs([job]) };
    }
    case 'jobs_control_all': {
      jobsHeld = args.action === 'stop';
      if (jobsHeld) for (const r of regionRequests.values()) r.cancelled = true;
      const changed: Job[] = [];
      const errors: { id: string; message: string }[] = [];
      const latest = new Map<string, Job>();
      const ongoing = new Map<string, Job>();
      for (const j of previewJobs) {
        const key = j.project + '/' + j.pageId;
        if (!latest.has(key) || latest.get(key)!.created < j.created) latest.set(key, j);
        if (['running', 'queued', 'paused'].includes(j.status)) ongoing.set(key, j);
      }
      for (const job of previewJobs) {
        const key = job.project + '/' + job.pageId;
        if (
          jobsHeld
            ? ['running', 'queued'].includes(job.status)
            : ['queued', 'paused', 'failed'].includes(job.status) &&
              (!ongoing.has(key) || ongoing.get(key) === job) &&
              (job.status !== 'failed' || latest.get(key) === job)
        ) {
          const status = !jobsHeld ? jobStatus(job) : null;
          if (status && !status.ready) {
            job.status = 'paused';
            job.error = status.issues.map((i) => i.message).join('\n');
            job.sequence = ++jobSequence;
            errors.push({ id: job.id, message: job.error });
            changed.push(job);
            continue;
          }
          updateJob(job, jobsHeld ? 'pause' : 'resume');
          changed.push(job);
        }
      }
      return {
        outcome: {
          changed: changed.length,
          skipped: previewJobs.length - changed.length,
          errors,
        },
        state: publishJobs(changed),
      };
    }
    case 'translation_batch_preview':
      return structuredClone(batchSnapshot(find(), args.chapterId));
    case 'translation_batch_submit': {
      await new Promise((resolve) => setTimeout(resolve, 120));
      const book = find(),
        s = effective(book.path);
      if (!['skip_translated', 'replace'].includes(args.mode))
        throw Error('Invalid translation mode');
      if (book.id !== args.bookId) throw Error('Book changed; reopen translation options');
      requireReady(actionStatus('translate', s));
      const expected = new Map<string, number>();
      for (const page of args.pages) {
        if (expected.has(page.id)) throw Error('Duplicate page in translation submission');
        expected.set(page.id, page.revision);
      }
      const snapshot = batchSnapshot(book, args.chapterId);
      let skipped = 0,
        omitted = 0,
        busy = 0,
        changed = 0;
      const added: Job[] = [];
      for (const p of snapshot.pages) {
        if (!expected.has(p.id)) continue;
        const revision = expected.get(p.id);
        expected.delete(p.id);
        if (p.revision !== revision) changed++;
        else if (p.omitted) omitted++;
        else if (args.mode === 'skip_translated' && p.translated) skipped++;
        else if (p.busy) busy++;
        else
          added.push(
            makePreviewJob(
              book,
              p.id,
              s,
              args.mode === 'replace',
              args.mode === 'replace' ? p.revision : null,
              'translation',
            ),
          );
      }
      changed += expected.size;
      previewJobs.push(...added);
      publishJobs(added);
      return structuredClone({
        jobs: added.map(jobView),
        warnings: [],
        skipped,
        omitted,
        busy,
        changed,
        held: jobsHeld,
      });
    }
    case 'restart_enqueue':
    case 'prepare_enqueue':
    case 'cleanup_enqueue':
    case 'enqueue': {
      await new Promise((resolve) => setTimeout(resolve, 120));
      const book = find();
      const added: Job[] = [];
      let untranslated = 0,
        busy = 0;
      const cleanup = command === 'cleanup_enqueue';
      const preparing = command === 'prepare_enqueue';
      const fresh = preparing || command === 'restart_enqueue';
      const s = effective(book.path);
      const operation = preparing ? 'prepare' : cleanup ? 'cleanup' : 'translate';
      const p = fresh ? state.pages[args.pageId] : undefined;
      const check = actionStatus(operation, s, p);
      if (cleanup) check.issues = check.issues.filter((i) => i.code !== 'regions');
      check.ready = !check.issues.length;
      requireReady(check);
      if (fresh && (!p || p.revision !== args.expected))
        throw Error('Page changed; confirm preparation again.');
      if (
        fresh &&
        (p!.regions.length || p!.rendered || p!.cleanup || p!.background) &&
        !args.replace
      )
        throw Error('Confirm replacement before preparing this page.');
      for (const id of new Set((fresh ? [args.pageId] : args.pageIds) as string[])) {
        if (command === 'enqueue' && book.omittedPageIds.includes(id)) continue;
        if ([...regionRequests.values()].some((r) => r.pageId === id))
          throw Error('This page already has an operation; wait for it to finish');
        if (cleanup && !state.pages[id]?.regions.some((r) => r.prepared || r.target.trim())) {
          untranslated++;
          continue;
        }
        if (
          previewJobs.some(
            (j) =>
              j.project === book.path &&
              j.pageId === id &&
              ['running', 'queued', 'paused'].includes(j.status),
          )
        ) {
          busy++;
          continue;
        }
        const chapter = book.chapters.find((c) => c.pageIds.includes(id));
        if (!chapter) continue;
        added.push(
          makePreviewJob(
            book,
            id,
            s,
            fresh,
            fresh ? args.expected : null,
            preparing ? 'preparation' : cleanup ? 'cleanup' : 'translation',
          ),
        );
      }
      previewJobs.push(...added);
      publishJobs(added);
      return structuredClone({
        jobs: added.map(jobView),
        warnings: [],
        held: jobsHeld,
        ...(cleanup ? { untranslated, busy } : {}),
      });
    }
    case 'book_refresh':
      return summaries().find((b) => b.path === args.path);
    case 'import_cancel':
      return;
    case 'import_scan': {
      const preview: ImportPreview = { chapters: [], omittedPageIds: [], pages: [], warnings: [] };
      const c = chapter('Preview chapter');
      for (let i = 0; i < 4; i++) {
        const p = page(`Page ${i + 1}.png`);
        preview.pages.push(p);
        c.pageIds.push(p.id);
      }
      preview.chapters.push(c);
      return preview;
    }
    case 'book_create': {
      const id = randomUuid();
      const b: Book = {
        id,
        path: `preview://${id}`,
        metadata: args.metadata,
        chapters: args.preview.chapters,
        omittedPageIds: args.preview.omittedPageIds,
        cover: args.cover,
        overrides: { translation: null, reader: null },
        glossary: emptyGlossary(),
        revision: 0,
      };
      for (const p of args.preview.pages) state.pages[p.id] = p;
      state.books.push(b);
      persist();
      return structuredClone(b);
    }
    case 'book_update': {
      const b = find();
      if (b.revision !== args.expected) throw Error('Book changed; reopen it before applying');
      if (args.coverChanged) b.cover = args.cover;
      b.metadata = args.metadata;
      b.overrides = args.overrides;
      b.revision++;
      persist();
      return structuredClone(b);
    }
    case 'chapter_complete': {
      const b = find();
      b.chapters.find((c) => c.id === args.chapterId)!.read = args.read;
      b.revision++;
      persist();
      return structuredClone(b);
    }
    case 'book_organize': {
      const b = find();
      if (b.revision !== args.expected) throw Error('Book changed; reopen the organizer');
      const retained = new Set<string>(
        args.chapters.flatMap((c: Book['chapters'][number]) => c.pageIds),
      );
      if (
        args.omittedPageIds.some((id: string) => !retained.has(id)) ||
        new Set(args.omittedPageIds).size !== args.omittedPageIds.length
      )
        throw Error('Invalid omitted pages');
      b.chapters = args.chapters;
      b.omittedPageIds = args.omittedPageIds;
      b.revision++;
      for (const p of args.added) state.pages[p.id] = p;
      for (const [number, id] of b.chapters.flatMap((c) => c.pageIds).entries()) {
        state.pages[id].number = number;
        state.pages[id].revision++;
      }
      persist();
      return structuredClone(b);
    }
    case 'book_delete':
      state.books = state.books.filter((b) => b.path !== args.path);
      persist();
      return;
    case 'edit_page': {
      if (
        [...regionRequests.values()].some((v) => v.path === args.path) ||
        previewJobs.some((j) => j.project === args.path && (j.status === 'running' || j.stopping))
      )
        throw Error('This book is processing another page; wait for it to finish');
      if ([...regionRequests.values()].some((r) => r.pageId === args.page.id))
        throw Error('This page already has an operation; wait for it to finish');
      const p = mergePageEdits(args.base, args.page, state.pages[args.page.id]);
      p.revision++;
      if (fixture === 'reader-cleanup') {
        // Only a UI fixture: the real mask/pixel algorithm is validated by native tests.
        p.status = 'edited';
        for (const r of p.regions) r.review = null;
      }
      state.pages[p.id] = p;
      persist();
      return structuredClone(p);
    }
    case 'library_clear_thumbnails':
      return 0;
    case 'catalog_refresh':
      return controlFixture ? controlCatalog : previewCatalog;
    default:
      throw Error('This action requires the U-Manga desktop application.');
  }
}

function checkModelLibrary(applied: Settings, library: string) {
  if (!applied.libraryDirectory || library !== applied.libraryDirectory)
    throw Error('Choose and apply the library folder before downloading or verifying models.');
}
