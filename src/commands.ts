/** The production IPC boundary. Argument names match Tauri's camelCase contract. */
import type * as T from './types';
type Command<A, R> = { args: A; result: R };
type Path = { path: string };
type PageKey = Path & { pageId: string };
type Model = { id: string; library: string };
export type JobAction = 'pause' | 'resume' | 'retry' | 'cancel';
export type Commands = {
  host_status: Command<undefined, import('./types').HostStatus>;
  host_start: Command<{ port: number; password: string | null }, import('./types').HostStatus>;
  host_stop: Command<undefined, import('./types').HostStatus>;
  host_qr: Command<{ url: string }, string>;
  system_language: Command<undefined, string>;
  onboarding_language: Command<{ language: import('./i18n').LanguagePreference }, void>;
  bootstrap: Command<
    undefined,
    {
      settings: T.Settings;
      systemLocale: string;
      models: T.ModelPack[];
      catalog: Record<string, any>;
      fonts: string[];
      jobsSmoke: boolean;
      jobs?: T.Job[];
    }
  >;
  preferences: Command<{ settings: T.Settings; base?: T.Settings }, T.Settings>;
  library_list: Command<undefined, T.BookSummary[]>;
  library_relocate: Command<{ destination: string }, T.Settings>;
  library_clear_thumbnails: Command<undefined, number>;
  book_open: Command<Path & { glossary?: boolean }, T.Book>;
  book_refresh: Command<Path, T.BookSummary>;
  book_import: Command<Path, T.Book>;
  book_delete: Command<Path, void>;
  book_sources: Command<Path, { id: string; source: T.Page['source'] }[]>;
  book_create: Command<
    { metadata: T.Metadata; preview: T.ImportPreview; cover: string | null },
    T.Book
  >;
  book_glossary_save: Command<Path & { expected: number; glossary: T.BookGlossary }, T.Book>;
  glossary_parse: Command<{ text: string; format: string }, T.GlossaryEntry[]>;
  glossary_export: Command<
    Path & { destination: string; entries: T.GlossaryEntry[]; format: string },
    void
  >;
  book_update: Command<
    Path & {
      metadata: T.Metadata;
      overrides: T.Book['overrides'];
      expected: number;
      coverChanged?: boolean;
      cover?: string | null;
    },
    T.Book
  >;
  book_organize: Command<
    Path & { expected: number; chapters: T.Chapter[]; added: T.Page[]; omittedPageIds: string[] },
    T.Book
  >;
  book_export: Command<
    Path & { chapterId: string | null; destination: string; format: string },
    number
  >;
  chapter_complete: Command<Path & { chapterId: string; read: boolean }, T.Book>;
  chapter_pages: Command<Path & { chapterId: string }, T.Project>;
  page_get: Command<PageKey, T.Page>;
  page_image: Command<PageKey & { translated: boolean }, string>;
  thumbnail: Command<PageKey, string>;
  source_thumbnail: Command<{ source: T.Page['source'] }, string>;
  edit_page: Command<Path & { page: T.Page; base: T.Page; expected: number }, T.Page>;
  import_scan: Command<{ id: string; paths: string[]; automatic: boolean }, T.ImportPreview>;
  import_cancel: Command<{ id: string }, void>;
  enqueue: Command<Path & { pageIds: string[]; priority?: boolean }, T.JobSubmission>;
  translation_batch_preview: Command<
    Path & { chapterId: string | null },
    T.TranslationBatchPreview
  >;
  translation_batch_submit: Command<
    Path & {
      bookId: string;
      chapterId: string | null;
      pages: T.PageVersion[];
      mode: T.TranslationBatchMode;
    },
    T.TranslationBatchResult
  >;
  prepare_enqueue: Command<PageKey & { expected: number; replace: boolean }, T.JobSubmission>;
  restart_enqueue: Command<PageKey & { expected: number; replace: boolean }, T.JobSubmission>;
  region_read: Command<{ request: T.RegionRequest }, T.RegionResult>;
  region_translate: Command<{ request: T.RegionRequest }, T.RegionResult>;
  region_cancel: Command<{ id: string }, void>;
  cleanup_enqueue: Command<
    Path & { pageIds: string[] },
    T.JobSubmission & { untranslated: number; busy: number }
  >;
  jobs_list: Command<undefined, T.JobsBatch>;
  job_control: Command<{ id: string; action: JobAction }, T.JobsControl>;
  jobs_control_all: Command<{ action: 'start' | 'stop' }, T.JobsControl>;
  jobs_readiness: Command<{ ids: string[] }, Record<string, T.Availability>>;
  action_readiness: Command<
    {
      path?: string;
      page?: T.Page | null;
      base?: T.Page | null;
      pageIds?: string[];
      actions?: T.ActionName[];
    },
    T.ActionReadiness
  >;
  translation_readiness: Command<{ settings: T.Settings; force?: boolean }, T.Readiness>;
  onboarding_save: Command<
    { settings: T.Settings; base?: T.Settings; step: string; complete: boolean },
    T.Settings
  >;
  onboarding_appearance: Command<{ appearance: T.AppearanceSettings }, void>;
  model_storage: Command<undefined, { library: string; directory: string }>;
  model_downloads: Command<undefined, string[]>;
  model_verify: Command<Model, boolean>;
  model_download: Command<Model, void>;
  model_cancel: Command<{ id: string }, void>;
  catalog_refresh: Command<undefined, Record<string, any>>;
  secret_set: Command<{ profile: T.Provider; value: string }, void>;
  secret_status: Command<{ profile: T.Provider }, boolean>;
  ollama_models: Command<{ endpoint: string; force?: boolean }, T.OllamaModels>;
  ollama_model_details: Command<
    { endpoint: string; model: string; force?: boolean },
    T.OllamaStatus
  >;
  thinking_capability: Command<{ profile: T.Provider }, T.ThinkingPolicy>;
  service_test: Command<{ settings: T.Settings }, { elapsedMs: number; target: string }>;
  prompt_defaults: Command<{ sourceLanguage: string }, T.InstructionOverrides>;
  prompt_preview: Command<
    { profile: T.Provider; settings: T.TranslationSettings; task: T.InstructionTask },
    T.InstructionPreview
  >;
  show_source: Command<Path, void>;
  show_notices: Command<undefined, void>;
  jobs_smoke_ui_report: Command<{ report: unknown }, void>;
};
