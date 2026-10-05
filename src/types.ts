import type { UiText } from './i18n';
export type HostStatus = {
  running: boolean;
  port: number;
  passwordConfigured: boolean;
  sessions: number;
  urls: string[];
  error: import('./i18n').UiMessage | null;
};
export type Filters = {
  brightness: number;
  contrast: number;
  warmth: number;
  saturation: number;
  grayscale: number;
  inversion: number;
};
export type ThemeProfile = {
  colors: Record<string, string>;
  font: string;
  scale: number;
  density: number;
  readerBackground: string;
  margin: number;
  gap: number;
  filters: Filters;
};
export type AppearanceSettings = {
  active: 'day' | 'night';
  day: ThemeProfile;
  night: ThemeProfile;
};
export type InstructionTask =
  'textTranslation' | 'simpleTextTranslation' | 'visionTranslation' | 'glossaryDetection';
export type InstructionOverrides = Record<InstructionTask, string | null>;
export type InstructionPreview = {
  prompt: string;
  system: string;
  user: string;
  schema: Record<string, unknown> | null;
};
export type Provider = {
  instructions: InstructionOverrides;
  simpleTranslation: boolean;
  thinking: boolean;
  thinkingPolicy?: ThinkingPolicy | null;
  id: string;
  name: string;
  service: string;
  protocol: string;
  endpoint: string;
  model: string;
  vision: boolean;
  region: string;
  appId: string;
  rateLimit: number;
};
export type Settings = {
  uiLanguage: import('./i18n').LanguagePreference;
  formatVersion: number;
  concurrentBooks: number;
  setup: { completed: boolean; step: string };
  appearance: AppearanceSettings;
  providers: Provider[];
  libraryDirectory: string;
  libraryView: 'grid' | 'card' | 'list';
  librarySort: string;
  reader: ReaderSettings;
  translation: TranslationSettings;
};
export type ReaderSettings = { layout: string; zoom: number };
export type Metadata = {
  title: string;
  creator: string;
  description: string;
  language: string;
  tags: string[];
};
export type Chapter = { id: string; title: string; pageIds: string[]; read: boolean };
export type GlossaryEntry = { source: string; target: string };
export type BookGlossary = {
  autoDetect: boolean;
  automaticSources: string[];
  enabled: boolean;
  entries: GlossaryEntry[];
  deeplGlossaryId: string;
  revision: number;
};
export type Book = {
  warnings?: import('./i18n').UiMessage[];
  glossary: BookGlossary;
  path: string;
  id: string;
  metadata: Metadata;
  chapters: Chapter[];
  omittedPageIds: string[];
  cover: string | null;
  overrides: { translation: TranslationSettings | null; reader: ReaderSettings | null };
  revision: number;
};
export type BookSummary = {
  path: string;
  id: string;
  metadata: Metadata;
  chapters: number;
  completed: number;
  pages: number;
  translated: number;
  review: number;
  cover: string | null;
  coverPage: string | null;
  updated: string;
};
export type ImportPreview = {
  chapters: Chapter[];
  omittedPageIds: string[];
  pages: Page[];
  warnings: UiText[];
};
export type TranslationBatchMode = 'skip_translated' | 'replace';
export type PageVersion = { id: string; revision: number };
export type TranslationBatchPreview = {
  bookId: string;
  bookTitle: string;
  chapterTitle: string | null;
  pages: (PageVersion & {
    number: number;
    translated: boolean;
    omitted: boolean;
    hasWork: boolean;
    busy: boolean;
  })[];
};
export type TranslationBatchResult = {
  jobs: Job[];
  warnings: { id: string; message: UiText }[];
  skipped: number;
  omitted: number;
  busy: number;
  changed: number;
  held: boolean;
};
export type JobSubmission = {
  jobs: Job[];
  held: boolean;
  warnings: { id: string; message: UiText }[];
};
export type CleanupSettings = {
  method: 'solid' | 'migan' | 'manga_aot' | 'manga_lama';
  strategy: 'automatic' | 'all_regions';
  device: 'auto' | 'cpu' | 'directml';
};
export type PageCleanup = {
  settings: CleanupSettings;
  key: string;
  path: string;
  device: string;
  reviews: Record<string, UiText>;
};
export type TranslationSettings = {
  sourceLanguage: string;
  targetLanguage: string;
  mode: string;
  ocr: string;
  device: string;
  providerId: string;
  glossaryEnabled: boolean;
  contextPages: number;
  glossary: GlossaryEntry[];
  deeplGlossaryId: string;
  autoGlossary: boolean;
  cleanup: CleanupSettings;
};
export type Region = {
  id: string;
  bbox: number[];
  bubble: number[] | null;
  kind: string;
  score: number;
  source: string;
  target: string;
  direction: string;
  style: {
    font: string | null;
    size: number | null;
    color: string;
    fill: string;
    lineGap: number;
    outlineEnabled: boolean;
    outlineWidthPercent: number;
    outlineColor: string;
  };
  allowFill: boolean;
  overlayOnly: boolean;
  prepared: boolean;
  review: UiText | null;
};
export type Page = {
  id: string;
  number: number;
  name: string;
  source: { path: string; kind: string; entry: string | null; index: number };
  width: number;
  height: number;
  pdfPoints?: [number, number] | null;
  sourceStamp?: string;
  fingerprint: string;
  revision: number;
  regions: Region[];
  rendered: string | null;
  background: string | null;
  cleanup: PageCleanup | null;
  status: string;
  error: UiText | null;
};
export type Project = {
  path: string;
  id: string;
  title: string;
  settings: TranslationSettings;
  pages: Page[];
  omittedPageIds: string[];
};
/** Durable preview fixture shape; production exposes only Job below. */
export type CapturedJob = {
  steps: {
    stage: string;
    status: 'running' | 'complete' | 'skipped' | 'warning' | 'failed';
    detail: UiText;
    error: UiText | null;
  }[];
  pageRevision: number | null;
  fresh?: boolean;
  glossaryCheckpoint?: {
    completed: boolean;
    processed: string[];
    eligible: string[] | null;
    skipped: number;
    added: number;
    warning: UiText | null;
  } | null;
  kind: 'translation' | 'cleanup' | 'preparation';
  timerRunning?: boolean;
  receivedAt?: number;
  cleanupModel?: CleanupSettings['method'] | null;
  modelDirectory: string;
  sequence?: number;
  stopping?: boolean;
  id: string;
  project: string;
  pageId: string;
  status: 'queued' | 'running' | 'paused' | 'failed' | 'cancelled' | 'complete';
  stage: string;
  stageDetail: UiText;
  error: UiText | null;
  created: string;
  elapsedMs: number;
  device?: string;
  settings: TranslationSettings;
  provider: Provider;
  location: {
    bookId: string;
    bookTitle: string;
    chapterId: string;
    chapterTitle: string;
    pageNumber: number;
    chapterPages: number;
  } | null;
};
export type Job = Omit<
  CapturedJob,
  'settings' | 'provider' | 'modelDirectory' | 'glossaryCheckpoint'
> & {
  settings: Pick<
    TranslationSettings,
    'sourceLanguage' | 'targetLanguage' | 'mode' | 'autoGlossary' | 'glossaryEnabled' | 'cleanup'
  >;
  provider: Pick<Provider, 'name' | 'model' | 'service'>;
  glossaryCheckpoint?: { added: number } | null;
  requirementsKey?: string;
};
export type JobsBatch = {
  history?: { offset: number; limit: number; total: number; more: boolean };
  jobs: Job[];
  removed: string[];
  held: boolean;
  recovering: boolean;
  errors: UiText[];
  generation: number;
  version: number;
};
export type Availability = {
  ready: boolean;
  issues: { code: string; section: string; message: UiText }[];
};
export type ActionName =
  'translate' | 'prepare' | 'cleanup' | 'edit' | 'service' | 'region_read' | 'region_translate';
export type RegionRequest = {
  id: string;
  path: string;
  pageId: string;
  expected: number;
  region: Region;
};
export type RegionResult = {
  id: string;
  pageId: string;
  regionId: string;
  expected: number;
  text: string;
  elapsedMs: number;
};
export type ActionReadiness = Record<ActionName, Availability>;
export type JobsOutcome = {
  changed: number;
  skipped: number;
  errors: { id: string; message: UiText }[];
};
export type JobsControl = { outcome: JobsOutcome; state: JobsBatch };
export type ModelPack = {
  description: string;
  distribution: 'bundled' | 'download';
  id: string;
  name: string;
  kind: string;
  languages: string[];
  license: string;
  revision: string;
  files: { name: string; url: string; sha256: string; bytes: number }[];
};
export type Readiness = {
  ready: boolean;
  advancement: Record<string, Availability>;
  credentialStored: boolean;
  credentialRequired: boolean;
  ollama: OllamaStatus | null;
  destination: string;
  issues: { code: string; section: string; message: UiText }[];
  packs: {
    id: string;
    name: string;
    distribution: string;
    bytes: number;
    status: string;
    path: string;
    required: boolean;
  }[];
};
export type OllamaModel = { name: string; size: number; digest: string; remote: boolean };
export type ThinkingPolicy = {
  state: 'switchable' | 'fixed_on' | 'fixed_off' | 'managed';
  on: string | number | null;
  off: string | number | null;
};
export type OllamaModelInfo = OllamaModel & { capabilities: string[]; thinking: ThinkingPolicy };
export type OllamaModels = {
  endpoint: string;
  connected: boolean;
  models: OllamaModel[];
  message: UiText | null;
};
export type OllamaStatus = {
  endpoint: string;
  connected: boolean;
  model: OllamaModelInfo | null;
  message: UiText | null;
};
