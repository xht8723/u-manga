<script lang="ts">
  import { onDestroy } from 'svelte';
  import { Notifications } from './notifications';
  import { provideNotifications } from './notification-context';
  import NotificationHost from './NotificationHost.svelte';
  import { modalStack } from './modal-stack';
  let { sessionCovered = false }: { sessionCovered?: boolean } = $props();
  const notifications = new Notifications();
  provideNotifications(notifications);
  let disposed = false;
  const stopWarningSession = beginWarningSession();
  const call = scopedCommands(() => () => !disposed);
  const notify = notifications.scope(() => !disposed);
  onDestroy(() => {
    disposed = true;
    stopWarningSession();
    notifications.destroy();
  });
  import { preferenceCoordinator, checkedRelocation } from './preferences';
  import {
    uiError,
    uiJoin,
    type UiText,
    t,
    tr,
    count,
    locale,
    setLanguage,
    updateSystemLanguage,
  } from './i18n';
  import { AsyncOwner, ownedResult } from './async-owner';
  import { LibraryPublication, libraryPath } from './library-publication';
  import { refreshQueue } from './refresh-queue';
  import { jobIndex } from './job-index';
  import { jobDestination } from './job-navigation';
  const navigation = new AsyncOwner();
  import ComboBox from './ComboBox.svelte';
  import { useCompactLayout } from './mobile-layout.svelte';
  import { visibleViewport, observeViewport } from './viewport';
  import GlossaryModal from './GlossaryModal.svelte';
  import TranslationBatchDialog from './TranslationBatchDialog.svelte';
  import { batchFeedback } from './batch-translation';
  import { GlossaryJobChanges } from './glossary-sync';
  import RequirementHint from './RequirementHint.svelte';
  import { actionReadiness } from './action-readiness.svelte';
  import { onMount, tick } from 'svelte';
  import { open } from '@tauri-apps/plugin-dialog';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { listen } from '@tauri-apps/api/event';
  import {
    Library,
    Layers,
    Settings2,
    Sun,
    Moon,
    Plus,
    Search,
    Grid2X2,
    PanelsTopLeft,
    List,
    ArrowLeft,
    BookOpen,
    Languages,
    Server,
    SlidersHorizontal,
  } from 'lucide-svelte';
  import type {
    Settings,
    Book,
    BookSummary,
    Chapter,
    Project,
    Job,
    ModelPack,
    Metadata,
  } from './types';
  import { settingsDefaults, naturalCompare, effectiveSettings } from './book-state';
  import { applyTheme } from './appearance';
  import { scopedCommands, beginWarningSession, native, hosted } from './bridge';
  import { listenContent } from './content-events';
  import HostDialog from './HostDialog.svelte';
  import WebPreferences from './WebPreferences.svelte';
  import StartupError from './StartupError.svelte';
  import { chooseLibrary, confirmDelete } from './files';
  import LibraryGrid from './LibraryGrid.svelte';
  import Cover from './Cover.svelte';
  import CreateBook from './CreateBook.svelte';
  import Organizer from './Organizer.svelte';
  import UnifiedSettings from './UnifiedSettings.svelte';
  import Onboarding from './Onboarding.svelte';
  import './setup.css';
  import BookDetailsFields from './BookDetailsFields.svelte';
  import ExportDialog from './ExportDialog.svelte';
  import Modal from './Modal.svelte';
  import Workspace from './Workspace.svelte';
  import JobsView from './JobsView.svelte';
  import { JobCollection, listenJobs } from './job-events';
  import type { JobsBatch, JobsControl } from './types';
  const compact = useCompactLayout();
  let libraryFiltersOpen = $state(false);
  $effect(() => {
    if (!compact.current) libraryFiltersOpen = false;
  });
  let settings = $state<Settings>(settingsDefaults()),
    models = $state<ModelPack[]>([]),
    catalog = $state<Record<string, any>>({}),
    fonts = $state<string[]>([]),
    jobs = $state.raw<Job[]>([]),
    books = $state<BookSummary[]>([]);
  const libraryPublication = new LibraryPublication<BookSummary>();
  const jobCollection = new JobCollection();
  const glossaryChanges = new GlossaryJobChanges();
  let jobsHeld = $state(false),
    jobsRecovering = $state(false),
    jobsErrors = $state<UiText[]>([]);
  const refreshBooks = new Set<string>();
  const refreshGlossaries = new Set<string>();
  let refreshWholeLibrary = false;
  let refreshTimer: ReturnType<typeof setTimeout> | undefined;
  const refreshBookSnapshot = refreshQueue(async (path: string) => {
    if (disposed) return;
    const generation = jobCollection.generation;
    try {
      const saved = await call('book_open', { path, glossary: true });
      if (!disposed && generation === jobCollection.generation) acceptBookSnapshot(saved);
    } catch {
      /* A later explicit event/reconnect retries failed reads. */
    }
  });
  const refreshLibrary = refreshQueue(async (directory: string) => {
    if (disposed || directory !== settings.libraryDirectory) return;
    try {
      await refresh();
    } catch {
      /* refresh publishes its scoped error. */
    }
  });
  let submitting = $state<string[]>([]);
  let translationTarget = $state<{ book: Book; chapterId: string | null } | null>(null);
  function syncLibrary() {
    if (!libraryPublication.activate(settings.libraryDirectory)) return;
    books = [];
    refreshBooks.clear();
    refreshGlossaries.clear();
    refreshWholeLibrary = false;
    clearTimeout(refreshTimer);
    refreshTimer = undefined;
  }
  function applyJobs(batch: JobsBatch) {
    syncLibrary();
    if (batch.generation < jobCollection.generation) return;
    if (hosted && batch.history) historyMore = batch.history.more;
    jobs = jobCollection.apply(batch);
    jobsHeld = jobCollection.held;
    jobsRecovering = jobCollection.recovering;
    jobsErrors = jobCollection.errors;
    for (const path of glossaryChanges.apply(batch))
      if (
        book?.path === path ||
        (modal === 'glossary' && modalBook?.path === path) ||
        (modal === 'translation' && translationTarget?.book.path === path)
      )
        refreshGlossaries.add(path);
    for (const j of batch.jobs) if (j.status === 'complete') refreshBooks.add(j.project);
    scheduleRefresh();
  }
  function scheduleRefresh() {
    if ((refreshWholeLibrary || refreshBooks.size || refreshGlossaries.size) && !refreshTimer)
      refreshTimer = setTimeout(() => {
        refreshTimer = undefined;
        const paths = [...refreshBooks];
        refreshBooks.clear();
        const glossaryPaths = [...refreshGlossaries];
        refreshGlossaries.clear();
        for (const path of glossaryPaths) void refreshBookSnapshot(path);
        if (refreshWholeLibrary) {
          refreshWholeLibrary = false;
          void refreshLibrary(settings.libraryDirectory);
          return;
        }
        for (const path of paths) {
          const target = books.find((b) => libraryPath(b.path) === libraryPath(path));
          if (!target) continue;
          const ticket = libraryPublication.beginBook(target.id, path);
          void call('book_refresh', { path })
            .then((summary) => {
              if (libraryPublication.acceptBook(ticket, summary)) {
                books = libraryPublication.values();
                libraryError = '';
              }
            })
            .catch(() => {
              // A committed remote deletion also invalidates a known book path.
              // Reconcile membership before presenting a vanished book as a read error.
              if (!disposed && libraryPublication.current(ticket))
                void refreshLibrary(settings.libraryDirectory);
            });
        }
      }, 400);
  }
  async function loadJobs() {
    applyJobs(await call('jobs_list'));
  }
  function acceptBookSnapshot(saved: Book) {
    // Late reads must not roll back another editor's newer book snapshot.
    if (book?.path === saved.path && saved.revision >= book.revision) book = saved;
    if (
      modal === 'glossary' &&
      modalBook?.path === saved.path &&
      saved.revision >= modalBook.revision
    )
      modalBook = saved;
    if (
      modal === 'translation' &&
      translationTarget?.book.path === saved.path &&
      saved.revision >= translationTarget.book.revision
    )
      translationTarget = { ...translationTarget, book: saved };
  }
  function glossarySaved(updated: Book) {
    libraryPublication.changed(updated.id);
    if (book?.path === updated.path) book = updated;
    if (modalBook?.path === updated.path) modalBook = updated;
    if (translationTarget?.book.path === updated.path)
      translationTarget = { ...translationTarget, book: updated };
    window.dispatchEvent(new Event('umanga-requirements-changed'));
    act(refresh);
  }
  let book = $state<Book | null>(null),
    reading = $state<Project | null>(null),
    chapterId = $state(''),
    screen = $state('Library'),
    modal = $state(''),
    modalBook = $state<Book | null>(null),
    ready = $state(false),
    busy = $state(false);
  let metadataError = $state<UiText>('');
  $effect(() =>
    notifications.cover('shell', sessionCovered || busy || !!modal || !ready || !!startupError),
  );
  $effect(() => notifications.cover('modal', $modalStack.length > 0));
  let viewSaving = $state(false);
  let startupError = $state<UiText>('');
  let libraryError = $state<UiText>('');
  let search = $state(''),
    sort = $state('title'),
    language = $state(''),
    tag = $state(''),
    completion = $state(''),
    editMetadata = $state<Metadata | null>(null),
    editCover = $state<string | null>(null),
    menu = $state<BookSummary | null>(null),
    chapterMenu = $state<Chapter | null>(null),
    exportChapter = $state<Chapter | null>(null),
    menuX = $state(0),
    menuY = $state(0),
    menuElement = $state<HTMLDivElement>();
  let menuTrigger: HTMLElement | null = null;
  const activeFilters = $derived([language, tag, completion].filter(Boolean).length);
  const readiness = actionReadiness(() => ({
    settings,
    path: menu?.path || book?.path,
    actions: menu || chapterMenu || screen === 'Book' ? ['translate'] : [],
  }));
  let hostingRunning = $state(false);
  let historyMore = $state(false),
    historyOffset = $state(50),
    loadingHistory = $state(false);
  async function loadHistory() {
    if (loadingHistory) return;
    loadingHistory = true;
    try {
      const batch = await (await import('./hosted')).nextJobs(historyOffset);
      applyJobs(batch);
      historyOffset += 50;
      historyMore = batch.history.more;
    } finally {
      loadingHistory = false;
    }
  }
  let settingsTab = $state('General');
  function translationSettings() {
    settingsTab = 'Translation';
    modal = 'settings';
  }
  let filtered = $derived(
    books
      .filter((b) => {
        const text =
          `${b.metadata.title} ${b.metadata.creator} ${b.metadata.tags.join(' ')}`.toLocaleLowerCase();
        return (
          text.includes(search.toLocaleLowerCase()) &&
          (!language || b.metadata.language === language) &&
          (!tag || b.metadata.tags.includes(tag)) &&
          (!completion ||
            (completion === 'read'
              ? b.chapters > 0 && b.completed === b.chapters
              : b.completed < b.chapters || !b.chapters))
        );
      })
      .sort((a, b) =>
        sort === 'updated'
          ? b.updated.localeCompare(a.updated)
          : naturalCompare(
              sort === 'creator' ? a.metadata.creator : a.metadata.title,
              sort === 'creator' ? b.metadata.creator : b.metadata.title,
            ),
      ),
  );
  let workspace = $state<Workspace>();
  let workspaceInitialPageId = $state<string | undefined>();
  let openingJob = $state('');
  let openingJobRequest = 0;
  async function editJob(job: Job) {
    if (openingJob === job.id) return;
    const request = ++openingJobRequest;
    const owned = navigation.begin();
    const library = settings.libraryDirectory;
    const current = () => owned() && !disposed && settings.libraryDirectory === library;
    openingJob = job.id;
    try {
      if (workspace && !(await workspace.beforeLeave())) return;
      if (!current()) return;
      const destination = await ownedResult(current, jobDestination(
        job.project, job.pageId,
        path => call('book_open', { path }),
        (path, chapterId) => call('chapter_pages', { path, chapterId }),
        current,
      ));
      if (!destination || !current()) return;
      await restoreWindow();
      if (!current()) return;
      readerCollapsed = false;
      workspaceMode = 'Editor';
      workspaceInitialPageId = destination.pageId;
      book = destination.book;
      chapterId = destination.chapterId;
      reading = destination.project;
      screen = 'Reader';
    } catch (e) {
      if (current()) notify('error', uiError(e));
    } finally {
      if (request === openingJobRequest) openingJob = '';
    }
  }
  let workspaceMode = $state('Reader'),
    readerCollapsed = $state(false),
    fullscreen = $state(false);
  let fullscreenBefore: boolean | null = null;
  async function actualFullscreen() {
    return native ? getCurrentWindow().isFullscreen() : !!document.fullscreenElement;
  }
  async function setFullscreen(enabled: boolean) {
    if (native) await getCurrentWindow().setFullscreen(enabled);
    else if (
      enabled &&
      !document.fullscreenElement &&
      document.fullscreenEnabled &&
      document.documentElement.requestFullscreen
    )
      await document.documentElement.requestFullscreen();
    else if (!enabled && document.fullscreenElement) await document.exitFullscreen();
    fullscreen = await actualFullscreen();
  }
  async function restoreWindow() {
    if (fullscreenBefore !== null) {
      const before = fullscreenBefore;
      fullscreenBefore = null;
      if ((await actualFullscreen()) !== before) await setFullscreen(before);
    }
  }
  async function toggleFullscreen() {
    const current = await actualFullscreen();
    if (fullscreenBefore === null) fullscreenBefore = current;
    await setFullscreen(!current);
  }
  async function workspaceModeChanged(next: string) {
    if (next === 'Editor') await restoreWindow();
    else {
      fullscreen = await actualFullscreen();
      fullscreenBefore = fullscreen;
    }
  }
  async function leave(next: string) {
    navigation.invalidate();
    if (next === screen) return;
    if (workspace && !(await workspace.beforeLeave())) return;
    await restoreWindow();
    reading = null;
    screen = next;
  }
  let activeChapter = $derived(book?.chapters.find((c) => c.id === chapterId));
  let readableChapters = $derived(book?.chapters.filter((c) => c.pageIds.length) ?? []);
  let unreadChapter = $derived(readableChapters.find((c) => !c.read));
  let continueReading = $derived(!!unreadChapter && readableChapters.some((c) => c.read));
  let startChapter = $derived(unreadChapter ?? readableChapters[0]);
  let nextChapter = $derived(
    book?.chapters
      .slice((book?.chapters.findIndex((c) => c.id === chapterId) ?? -1) + 1)
      .find((c) => c.pageIds.length),
  );
  let appearancePreview = $state<Settings['appearance'] | null>(null);
  let displayedSettings = $derived(
    appearancePreview ? { ...settings, appearance: appearancePreview } : settings,
  );
  let readerSettings = $derived.by(() => {
    const translation = book
      ? effectiveSettings($state.snapshot(book), $state.snapshot(settings))
      : settings.translation;
    return {
      ...displayedSettings,
      translation: {
        ...translation,
        sourceLanguage: book?.metadata.language || translation.sourceLanguage,
      },
      reader: book?.overrides.reader
        ? { ...book.overrides.reader, zoom: settings.reader.zoom }
        : settings.reader,
    };
  });
  async function act(fn: () => Promise<unknown>) {
    let current = navigation.capture();
    try {
      const pending = fn();
      current = navigation.capture();
      await pending;
    } catch (e) {
      if (jobsSmoke) void call('jobs_smoke_ui_report', { report: { error: uiError(e) } });
      if (disposed || !current()) return;
      busy = false;
      if (!ready) {
        ready = true;
        startupError = uiError(e);
        if (native) await getCurrentWindow().show();
      } else notify('error', uiError(e));
    }
  }
  async function refresh() {
    syncLibrary();
    const ticket = libraryPublication.beginFull();
    try {
      const result = await call('library_list');
      if (!libraryPublication.acceptFull(ticket, result)) return;
      books = libraryPublication.values();
      libraryError = '';
    } catch (e) {
      if (!libraryPublication.current(ticket)) return;
      libraryError = uiError(e);
      throw e;
    }
  }
  const commitPreferences = preferenceCoordinator(
    (settings, base) => call('preferences', { settings, base }),
    (saved) => {
      settings = saved;
      syncLibrary();
      sort = saved.librarySort;
      applyTheme(saved.appearance);
      setLanguage(saved.uiLanguage);
    },
  );
  async function preferences(value: Settings, base = $state.snapshot(settings)) {
    const changedLibrary = settings.libraryDirectory !== value.libraryDirectory;
    const { uiLanguage: oldLanguage, ...oldSettings } = $state.snapshot(settings);
    const { uiLanguage: newLanguage, ...newSettings } = value;
    const languageOnly =
      oldLanguage !== newLanguage && JSON.stringify(oldSettings) === JSON.stringify(newSettings);
    if (changedLibrary && workspace && !(await workspace.beforeLeave()))
      throw Error('Library change cancelled; finish the current page edits first.');
    await commitPreferences(value, base);
    if (!languageOnly) {
      try {
        await refresh();
      } catch (e) {
        notify('error', uiError(e));
      }
    }
    if (changedLibrary) {
      navigation.invalidate();
      await restoreWindow();
      reading = null;
      book = null;
      screen = 'Library';
      await loadJobs();
    }
  }
  async function relocateLibrary(destination: string): Promise<Settings> {
    let warning: unknown;
    const saved = await commitPreferences.perform(async () => {
      const result = await checkedRelocation(
        settings.libraryDirectory,
        () => (workspace ? workspace.beforeLeave() : Promise.resolve(true)),
        () => call('library_relocate', { destination }),
        async () => (await call('bootstrap')).settings,
      );
      warning = result.warning;
      return result.settings;
    });
    // The native activation is already durable. No second cancellable draft
    // decision or preferences write may run against the retired old library.
    navigation.invalidate();
    reading = null;
    book = null;
    screen = 'Library';
    try {
      await restoreWindow();
      await refresh();
      await loadJobs();
    } catch (e) {
      notify('error', uiError(e));
    }
    if (warning !== undefined) throw warning;
    return saved;
  }
  async function setLibraryView(view: Settings['libraryView']) {
    if (viewSaving || view === settings.libraryView) return;
    viewSaving = true;
    try {
      const next = { ...$state.snapshot(settings), libraryView: view };
      await commitPreferences(next, $state.snapshot(settings));
    } finally {
      viewSaving = false;
    }
  }
  async function updatedBook(value: Book) {
    libraryPublication.changed(value.id);
    if (book?.path === value.path && value.revision >= book.revision) book = value;
    if (modalBook?.path === value.path) {
      modalBook = value;
      modal = '';
    }
    try {
      await refresh();
    } catch (e) {
      notify('error', uiError(e));
    }
  }
  async function switchTheme() {
    const s = structuredClone($state.snapshot(settings));
    s.appearance.active = s.appearance.active === 'day' ? 'night' : 'day';
    await preferences(s);
  }
  async function openBook(b: { path: string }) {
    const current = navigation.begin();
    const opened = await ownedResult(current, call('book_open', { path: b.path }));
    if (!opened) return;
    book = opened;
    reading = null;
    screen = 'Book';
    menu = null;
  }
  async function openGlossary() {
    if (!book) return;
    const current = navigation.begin();
    const saved = await ownedResult(current, call('book_open', { path: book.path }));
    if (!saved) return;
    modalBook = saved;
    modal = 'glossary';
  }
  async function openOrganizer() {
    if (!book) return;
    const current = navigation.begin();
    const saved = await ownedResult(current, call('book_open', { path: book.path }));
    if (!saved) return;
    acceptBookSnapshot(saved);
    modalBook = saved;
    modal = 'organizer';
  }
  async function setLibrary() {
    const path = await chooseLibrary();
    if (path) await preferences({ ...$state.snapshot(settings), libraryDirectory: path });
  }
  async function addBook() {
    if (!settings.libraryDirectory) {
      await setLibrary();
      return;
    }
    modal = 'create';
  }
  async function imported(b: Book) {
    syncLibrary();
    libraryPublication.created(b.id);
    book = b;
    reading = null;
    screen = 'Book';
    modal = '';
    await refresh();
  }
  async function importExisting() {
    if (!settings.libraryDirectory) {
      await setLibrary();
      if (!settings.libraryDirectory) return;
    }
    const path = await open({
      title: tr('Import U-Manga book'),
      filters: [{ name: 'U-Manga book', extensions: ['umanga'] }],
    });
    if (typeof path === 'string') {
      busy = true;
      await imported(await call('book_import', { path }));
      busy = false;
    }
  }
  async function read(c: Chapter, mode: 'Reader' | 'Editor' = 'Reader') {
    if (!book || !c.pageIds.length) return;
    const target = book.path;
    const current = navigation.begin();
    if (workspace && !(await workspace.beforeLeave())) return;
    if (!current()) return;
    const pages = await ownedResult(
      current,
      call('chapter_pages', { path: target, chapterId: c.id }),
    );
    if (!pages) return;
    if (screen !== 'Reader') {
      readerCollapsed = false;
      fullscreen = await actualFullscreen();
      fullscreenBefore = fullscreen;
    }
    if (mode === 'Editor') await restoreWindow();
    if (!current()) return;
    workspaceMode = mode;
    workspaceInitialPageId = undefined;
    reading = null;
    chapterId = c.id;
    reading = pages;
    screen = 'Reader';
  }
  async function complete(c: Chapter, value: boolean) {
    if (!book) return;
    const target = book.path;
    libraryPublication.changed(book.id);
    const updated = await call('chapter_complete', { path: target, chapterId: c.id, read: value });
    libraryPublication.changed(updated.id);
    if (book?.path === target && updated.revision >= book.revision) book = updated;
    await refresh();
  }
  async function translate(b: Book, chapter?: Chapter) {
    const scope = b.path + '/' + (chapter?.id || 'all');
    if (submitting.includes(scope)) return;
    if (!(chapter ? chapter.pageIds.length : b.chapters.some((c) => c.pageIds.length))) return;
    translationTarget = { book: $state.snapshot(b), chapterId: chapter?.id ?? null };
    modal = 'translation';
  }
  function showExport(chapter: Chapter | null = null) {
    exportChapter = chapter;
    modalBook = book;
    modal = 'export';
  }
  async function applyMetadata() {
    if (!modalBook || !editMetadata || busy) return;
    const target = modalBook;
    libraryPublication.changed(target.id);
    const metadata = $state.snapshot(editMetadata);
    const cover = editCover;
    metadataError = '';
    busy = true;
    try {
      const updated = await call('book_update', {
        path: target.path,
        metadata,
        overrides: $state.snapshot(target.overrides),
        expected: target.revision,
        coverChanged: cover !== target.cover,
        cover,
      });
      libraryPublication.changed(updated.id);
      if (book?.path === target.path) book = updated;
      if (modalBook?.path === target.path && modal === 'metadata') {
        modalBook = updated;
        editCover = updated.cover;
        modal = '';
      }
      await refresh();
    } catch (error) {
      if (!disposed && modal === 'metadata' && modalBook?.path === target.path)
        metadataError = uiError(error);
    } finally {
      busy = false;
    }
  }
  async function remove(b: Pick<Book, 'path' | 'metadata'>) {
    if (!(await confirmDelete([b.metadata.title]))) return;
    const summary = books.find((item) => libraryPath(item.path) === libraryPath(b.path));
    if (summary) libraryPublication.changed(summary.id);
    await call('book_delete', { path: b.path });
    if (summary) {
      libraryPublication.remove(summary.id);
      books = libraryPublication.values();
    }
    if (book?.path === b.path) {
      book = null;
      reading = null;
      screen = 'Library';
    }
    await refresh();
  }
  type MenuAction =
    | 'mark'
    | 'edit'
    | 'translate'
    | 'glossary'
    | 'export'
    | 'metadata'
    | 'chapters'
    | 'settings'
    | 'source'
    | 'delete';
  const menuItems = $derived<
    { action: MenuAction; label: string; disabled?: boolean; title?: UiText }[]
  >(
    (chapterMenu
      ? [
          ...(compact.current
            ? [{ action: 'edit', label: 'Edit', disabled: !chapterMenu.pageIds.length }]
            : []),
          { action: 'mark', label: chapterMenu.read ? 'Mark unread' : 'Mark read' },
          {
            action: 'translate',
            label: 'Translate chapter',
            disabled:
              !chapterMenu.pageIds.length ||
              !readiness.ready('translate') ||
              submitting.includes(`${book?.path}/${chapterMenu.id}`),
            title: readiness.reason('translate') || 'Choose how to translate this chapter',
          },
          { action: 'export', label: 'Export', disabled: !chapterMenu.pageIds.length },
        ]
      : [
          { action: 'metadata', label: 'Edit metadata', title: 'Edit book details and cover' },
          { action: 'chapters', label: 'Manage chapters' },
          { action: 'settings', label: 'Book settings' },
          { action: 'glossary', label: 'Glossary', title: 'Inspect and edit terms for this book' },
          {
            action: 'translate',
            label: 'Translate full book',
            disabled:
              !menu?.pages ||
              !readiness.ready('translate') ||
              submitting.includes(menu?.path + '/all'),
            title: readiness.reason('translate') || 'Choose how to translate this book',
          },
          { action: 'export', label: 'Export', disabled: !menu?.pages },
          { action: 'source', label: 'Show source files' },
          { action: 'delete', label: 'Delete' },
        ]) as { action: MenuAction; label: string; disabled?: boolean; title?: UiText }[],
  );
  async function menuAction(action: MenuAction) {
    const current = navigation.begin();
    const target = menu;
    const targetChapter = chapterMenu;
    const targetBook = book;
    dismissMenu();
    if (targetChapter && targetBook) {
      switch (action) {
        case 'edit':
          await read(targetChapter, 'Editor');
          break;
        case 'mark':
          await complete(targetChapter, !targetChapter.read);
          break;
        case 'translate':
          await translate(targetBook, targetChapter);
          break;
        case 'export':
          showExport(targetChapter);
          break;
      }
      return;
    }
    if (!target) return;
    if (action === 'delete') {
      await remove(target);
      return;
    }
    const opened = await ownedResult(
      current,
      call('book_open', { path: target.path, glossary: action === 'glossary' }),
    );
    if (!opened) return;
    modalBook = opened;
    switch (action) {
      case 'translate':
        await translate(opened);
        break;
      case 'glossary':
        modal = 'glossary';
        break;
      case 'metadata':
        metadataError = '';
        editMetadata = structuredClone($state.snapshot(opened.metadata));
        editCover = opened.cover;
        modal = 'metadata';
        break;
      case 'chapters':
        modal = 'organizer';
        break;
      case 'settings':
        modal = 'book-settings';
        break;
      case 'export':
        exportChapter = null;
        modal = 'export';
        break;
      case 'source': {
        const c = opened.chapters.find((c) => c.pageIds.length);
        if (c) {
          const sources = await call('book_sources', { path: opened.path });
          const source = sources.find((p) => p.id === c.pageIds[0]);
          if (source) await call('show_source', { path: source.source.path });
        } else await call('show_source', { path: opened.path });
        break;
      }
    }
  }
  let menuAnchor = { x: 8, y: 8 };
  function clampMenu() {
    if (!menuElement) return;
    const rect = menuElement.getBoundingClientRect();
    const view = visibleViewport();
    menuElement.style.maxHeight = `${Math.max(44, view.height - 16)}px`;
    menuElement.style.maxWidth = `${Math.max(44, view.width - 16)}px`;
    menuX = Math.max(view.left + 8, Math.min(menuAnchor.x, view.right - rect.width - 8));
    menuY = Math.max(view.top + 8, Math.min(menuAnchor.y, view.bottom - rect.height - 8));
  }
  function watchMenu(node: HTMLElement) {
    const observer = new ResizeObserver(clampMenu);
    observer.observe(node);
    const unwatch = observeViewport(clampMenu);
    return {
      destroy() {
        observer.disconnect();
        unwatch();
      },
    };
  }
  function positionMenu(e: MouseEvent) {
    e.preventDefault();
    e.stopPropagation();
    const target = e.target as HTMLElement;
    menuTrigger =
      target.closest<HTMLElement>('button') ||
      (e.currentTarget as HTMLElement).querySelector<HTMLElement>('button') ||
      (e.currentTarget as HTMLElement);
    const rect = menuTrigger.getBoundingClientRect();
    menuAnchor = { x: e.clientX || rect.left, y: e.clientY || rect.bottom };
    void tick().then(() => {
      clampMenu();
      menuElement?.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus();
    });
  }
  function showMenu(b: BookSummary, e: MouseEvent) {
    menu = b;
    chapterMenu = null;
    positionMenu(e);
  }
  function showChapterMenu(c: Chapter, e: MouseEvent) {
    e.preventDefault();
    menu = null;
    chapterMenu = c;
    positionMenu(e);
  }
  function dismissMenu() {
    menu = null;
    chapterMenu = null;
    menuTrigger?.focus({ preventScroll: true });
  }
  function menuKey(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      dismissMenu();
      return;
    }
    if (['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(e.key)) {
      e.preventDefault();
      const list = Array.from(
        menuElement!.querySelectorAll<HTMLButtonElement>('button:not(:disabled)'),
      );
      const at = list.indexOf(document.activeElement as HTMLButtonElement);
      list[
        e.key === 'Home'
          ? 0
          : e.key === 'End'
            ? list.length - 1
            : (at + (e.key === 'ArrowDown' ? 1 : -1) + list.length) % list.length
      ]?.focus();
    }
    if (e.key === 'Tab') dismissMenu();
  }
  let jobsSmoke = false;
  onMount(() => {
    let stop: undefined | (() => void);
    let stopContent: undefined | (() => void);
    let stopHosting: undefined | (() => void);
    let stopResize: undefined | (() => void);
    let stopSmoke: undefined | (() => void);
    let live = true;
    if (native)
      void listen<boolean>('hosting-changed', (event) => {
        if (live) hostingRunning = event.payload;
      }).then((stop) => {
        if (live) stopHosting = stop;
        else stop();
      });
    const syncFullscreen = () => {
      void actualFullscreen().then((value) => {
        if (live) fullscreen = value;
      });
    };
    const syncLanguage = () => {
      void call('system_language')
        .then((system) => {
          if (live) updateSystemLanguage(system);
        })
        .catch(() => {});
    };
    const commitWarning = (event: Event) => {
      notify('warning', (event as CustomEvent<UiText>).detail);
    };
    const content = (changedPath?: string | null) => {
      syncLibrary();
      if (changedPath && books.some((b) => libraryPath(b.path) === libraryPath(changedPath)))
        refreshBooks.add(changedPath);
      else refreshWholeLibrary = true;
      const paths = new Set(
        [
          book?.path,
          modal === 'glossary' ? modalBook?.path : undefined,
          modal === 'translation' ? translationTarget?.book.path : undefined,
        ].filter((path): path is string => !!path),
      );
      for (const path of paths)
        if (!changedPath || libraryPath(path) === libraryPath(changedPath))
          refreshGlossaries.add(path);
      scheduleRefresh();
    };
    const resyncContent = () => content();
    window.addEventListener('umanga-host-resync', resyncContent);
    void listenContent((change) => content(change.path)).then((stop) => {
      if (live) stopContent = stop;
      else stop();
    });
    window.addEventListener('umanga-commit-warning', commitWarning);
    window.addEventListener('focus', syncLanguage);
    document.addEventListener('fullscreenchange', syncFullscreen);
    act(async () => {
      const b = await call('bootstrap');
      settings = b.settings;
      if (native)
        void call('host_status')
          .then((s) => (hostingRunning = s.running))
          .catch(() => {});
      setLanguage(settings.uiLanguage, b.systemLocale);
      jobsSmoke = b.jobsSmoke && native;
      if (jobsSmoke) {
        screen = 'Jobs';
        stopSmoke = await (await import('./jobs-smoke')).startJobsSmoke();
      }
      models = b.models;
      catalog = b.catalog;
      fonts = b.fonts;
      jobs = b.jobs || [];
      sort = settings.librarySort;
      applyTheme(settings.appearance);
      if (settings.setup.completed) {
        try {
          await refresh();
        } catch {
          /* Keep Settings accessible for an unavailable or unsupported library. */
        }
      }
      if (jobsSmoke) await call('jobs_smoke_ui_report', { report: { stage: 'library loaded' } });
      ready = true;
      await tick();
      if (native) {
        await getCurrentWindow().show();
        stopResize = await getCurrentWindow().onResized(syncFullscreen);
      }
      stop = await listenJobs((batch) => {
        if (live) applyJobs(batch);
      });
      await loadJobs();
    });
    return () => {
      live = false;
      clearTimeout(refreshTimer);
      stop?.();
      stopResize?.();
      stopSmoke?.();
      stopContent?.();
      stopHosting?.();
      window.removeEventListener('umanga-host-resync', resyncContent);
      document.removeEventListener('fullscreenchange', syncFullscreen);
      window.removeEventListener('focus', syncLanguage);
      window.removeEventListener('umanga-commit-warning', commitWarning);
    };
  });
</script>

{#snippet bookActions()}{#if book}
    <div class="book-actions">
      <button
        class="primary"
        disabled={!startChapter}
        title={tr(
          startChapter
            ? `Read ${startChapter.title} from the first page`
            : 'Add chapter pages to start reading',
          $locale,
        )}
        onclick={() => startChapter && act(() => read(startChapter))}
        ><BookOpen size={18} />{continueReading
          ? t('m_4f1971cc1fe4', $locale)
          : t('m_9b9a8d05a7ec', $locale)}</button
      >
      <button
        class="translate-book"
        disabled={!book.chapters.some((c) => c.pageIds.length) ||
          !readiness.ready('translate') ||
          submitting.includes(book.path + '/all')}
        title={tr(readiness.reason('translate') || 'Choose how to translate this book', $locale)}
        onclick={() => book && act(() => translate(book!))}
        ><Languages size={18} />{tr('Translate full book', $locale)}</button
      >
    </div>
    <RequirementHint reason={readiness.hint('translate')} onsettings={translationSettings} />
  {/if}{/snippet}
{#snippet libraryViews()}<div
    class="view-switcher"
    role="group"
    aria-label={t('m_54570559b6b4', $locale)}
  >
    <button
      title={t('m_f358ee01acde', $locale)}
      aria-label={t('m_47da4e5507b6', $locale)}
      aria-pressed={settings.libraryView === 'grid'}
      disabled={viewSaving}
      onclick={() => act(() => setLibraryView('grid'))}><Grid2X2 size={17} /></button
    >
    <button
      title={t('m_3c17bbf1a204', $locale)}
      aria-label={t('m_c1477c8c3455', $locale)}
      aria-pressed={settings.libraryView === 'card'}
      disabled={viewSaving}
      onclick={() => act(() => setLibraryView('card'))}><PanelsTopLeft size={17} /></button
    >
    <button
      title={t('m_420b26b949da', $locale)}
      aria-label={t('m_5d8c3e1b635e', $locale)}
      aria-pressed={settings.libraryView === 'list'}
      disabled={viewSaving}
      onclick={() => act(() => setLibraryView('list'))}><List size={17} /></button
    >
  </div>{/snippet}
{#snippet libraryChoices()}<ComboBox
    label={tr('Sort books', $locale)}
    bind:value={sort}
    options={[
      { value: 'title', label: 'Title', disabled: false },
      { value: 'creator', label: 'Creator', disabled: false },
      { value: 'updated', label: 'Recently changed', disabled: false },
    ]}
  /><ComboBox
    label={tr('Filter by language', $locale)}
    bind:value={language}
    options={[
      { value: '', label: 'All languages', disabled: false },
      ...[...new Set(books.map((b) => b.metadata.language).filter(Boolean))]
        .sort()
        .flatMap((l) => [{ value: l, label: String(l).trim(), disabled: false }]),
    ]}
  /><ComboBox
    label={tr('Filter by tag', $locale)}
    bind:value={tag}
    options={[
      { value: '', label: 'All tags', disabled: false },
      ...[...new Set(books.flatMap((b) => b.metadata.tags))].sort().flatMap((t) => [
        {
          value: String(t).trim(),
          label: String(t).trim(),
          literal: true,
          disabled: false,
        },
      ]),
    ]}
  /><ComboBox
    label={tr('Filter by completion', $locale)}
    bind:value={completion}
    options={[
      { value: '', label: 'All books', disabled: false },
      { value: 'unread', label: 'Unread chapters', disabled: false },
      { value: 'read', label: 'Completed', disabled: false },
    ]}
  />{/snippet}

<svelte:window
  onkeydown={(e) => {
    if (e.key === 'Escape') dismissMenu();
  }}
/>
{#if startupError}<StartupError message={startupError} />
{:else if ready && !settings.setup.completed}<Onboarding
    value={settings}
    {models}
    {catalog}
    oncomplete={(next) => {
      settings = next;
      sort = next.librarySort;
      act(refresh);
    }}
  />
{:else if ready}<div
    class="library-app"
    class:compact-host={compact.current}
    tabindex="-1"
    data-notification-return
  >
    {#if screen !== 'Reader' || (workspaceMode === 'Editor' && !compact.current)}<header
        class="library-topbar"
      >
        <button
          class="wordmark"
          hidden={compact.current}
          title={t('m_008e42d22277', $locale)}
          onclick={() => act(() => leave('Library'))}>U<span>—</span>MANGA</button
        >
        <nav aria-label={t('m_eb355944b92d', $locale)}>
          <button class:active={screen === 'Library'} onclick={() => act(() => leave('Library'))}
            ><Library size={17} />{t('m_dc20b3d5d2cd', $locale)}</button
          ><button
            class:active={screen === 'Jobs'}
            onclick={() =>
              act(async () => {
                await leave('Jobs');
                await loadJobs();
              })}
            ><Layers size={17} />{t(
              'm_2f17a0f8d518',
              $locale,
            )}{#if jobs.some((j) => ['running', 'queued'].includes(j.status))}<span
                class="queue-count">{jobIndex(jobs).runningOrWaiting}</span
              >{/if}</button
          >
        </nav>
        <span class="spacer"></span><button
          class="theme-switch"
          aria-label={t(
            settings.appearance.active === 'day' ? 'm_8f2364e11b8b' : 'm_4e9f8db8242b',
            $locale,
          )}
          hidden={compact.current}
          title={tr(
            settings.appearance.active === 'day'
              ? 'Switch to Night appearance'
              : 'Switch to Day appearance',
            $locale,
          )}
          onclick={() => act(switchTheme)}
          >{#if settings.appearance.active === 'day'}<Sun size={17} />{:else}<Moon
              size={17}
            />{/if}</button
        ><button
          class="app-settings-button"
          aria-label={t('mobile.settings', $locale)}
          title={t('m_5ddd2ed2ba9a', $locale)}
          onclick={() => (modal = 'settings')}><Settings2 size={17} /></button
        >
      </header>{/if}
    {#if screen === 'Library'}<main class="library-screen">
        {#if libraryError}<p class="error-text" role="alert">{tr(libraryError, $locale)}</p>{/if}
        <div class="library-heading">
          <div>
            <h1>{t('m_dc20b3d5d2cd', $locale)}</h1>
            <small>{count(books.length, 'm_d08e2615a0ff', 'm_188586bcd737', $locale)}</small>
          </div>
          <span class="spacer"></span>{#if native}<button
              class="host-server-button"
              onclick={() => (modal = 'hosting')}
              title={t('host.intro', $locale)}
              ><Server size={17} />{t(
                hostingRunning ? 'host.running' : 'host.title',
                $locale,
              )}{#if hostingRunning}<span class="hosting-dot" aria-hidden="true"
                ></span>{/if}</button
            >{/if}{#if settings.libraryDirectory && !hosted}{#if native}<button
                title={t('m_da869a4e9fac', $locale)}
                onclick={() => act(importExisting)}>{t('m_f0648d87ebc4', $locale)}</button
              >{/if}<button
              class="primary"
              title={t('m_9560d660cbc0', $locale)}
              onclick={() => act(addBook)}><Plus size={18} />{t('m_c271b26bf773', $locale)}</button
            >{/if}
        </div>
        {#if !settings.libraryDirectory}<div class="empty library-empty library-setup">
            <Library size={44} />
            <h2>{t('m_b5899df2c5d1', $locale)}</h2>
            <p>{t('m_f9d7af90f304', $locale)}</p>
            <button
              class="primary"
              title={t('m_4fcc646ef433', $locale)}
              onclick={() => act(setLibrary)}>{t('m_887d1e4c2f99', $locale)}</button
            >
          </div>{:else}
          <div class="library-toolbar">
            {#if !compact.current}{@render libraryViews()}{/if}
            <div class="search-field">
              <Search size={17} /><input
                aria-label={t('m_1644de159307', $locale)}
                placeholder={t('m_4b3c32a761cd', $locale)}
                bind:value={search}
              />
            </div>
            {#if compact.current}<button
                class="library-filter-toggle"
                aria-haspopup="dialog"
                aria-expanded={libraryFiltersOpen}
                onclick={() => (libraryFiltersOpen = true)}
                ><SlidersHorizontal size={18} />{t(
                  'mobile.filters',
                  $locale,
                )}{#if activeFilters}<span class="filter-count">{activeFilters}</span>{/if}</button
              >{:else}{@render libraryChoices()}{/if}
          </div>
          {#if filtered.length}<LibraryGrid
              compact={compact.current}
              filterKey={`${search}/${sort}/${language}/${tag}/${completion}`}
              scale={displayedSettings.appearance[displayedSettings.appearance.active].scale}
              books={filtered}
              view={settings.libraryView}
              onopen={(b) => act(() => openBook(b))}
              onmenu={showMenu}
            />{:else}<div class="empty library-empty">
              <Library size={44} />
              <h2>{books.length ? t('m_1ec415261fe5', $locale) : t('m_e3176dfa0978', $locale)}</h2>
              {#if !books.length && !hosted}<button
                  class="primary"
                  title={t('m_9560d660cbc0', $locale)}
                  onclick={() => act(addBook)}>{t('m_c271b26bf773', $locale)}</button
                >{:else}<button
                  onclick={() => {
                    search = '';
                    language = '';
                    tag = '';
                    completion = '';
                  }}>{t('m_7179ea0035fc', $locale)}</button
                >{/if}
            </div>{/if}
        {/if}
      </main>
    {:else if screen === 'Book' && book}<main class="book-screen">
        <nav class="book-navigation" aria-label={t('m_f5d3e60b079a', $locale)}>
          <button
            class="library-return"
            title={t('m_008e42d22277', $locale)}
            onclick={() => act(() => leave('Library'))}
            ><ArrowLeft size={17} />{t('m_dc20b3d5d2cd', $locale)}</button
          >
        </nav>
        <section class="book-details">
          <div class="detail-cover">
            <Cover
              path={book.path}
              cover={book.cover}
              pageId={book.chapters.find((c) => c.pageIds.length)?.pageIds[0]}
              title={book.metadata.title}
            />
          </div>
          <div class="book-info">
            <div class="row">
              <h1>{book.metadata.title}</h1>
              <span class="spacer"></span><button
                title={t('m_077f42456a7a', $locale)}
                aria-haspopup="menu"
                aria-label={t('m_077f42456a7a', $locale)}
                onclick={(e) => {
                  const b = books.find((b) => b.id === book!.id);
                  if (b) showMenu(b, e);
                }}>•••</button
              >
            </div>
            {#if book.metadata.creator}<p class="book-creator">{book.metadata.creator}</p>{/if}
            <p class="book-description">{book.metadata.description}</p>
            <div class="row">
              <span>{book.chapters.length} {t('m_18ab26282a64', $locale)}</span
              >{#if book.metadata.language}<span class="tag">{book.metadata.language}</span
                >{/if}{#each book.metadata.tags as t}<span class="tag">{t}</span>{/each}
            </div>
            {#if !compact.current}{@render bookActions()}{/if}
          </div>
          {#if compact.current}<div class="compact-book-actions">{@render bookActions()}</div>{/if}
        </section>
        <section class="chapters-section">
          <h2>{t('m_9831375bc651', $locale)}</h2>
          {#each book.chapters as c, i (c.id)}<article
              class="chapter-detail"
              oncontextmenu={(e) => showChapterMenu(c, e)}
            >
              <span class="chapter-order">{String(i + 1).padStart(2, '0')}</span><button
                class="chapter-open"
                title={tr(
                  !c.pageIds.length
                    ? 'This chapter has no pages'
                    : `Read ${c.title} from the first page`,
                  $locale,
                )}
                disabled={!c.pageIds.length}
                onclick={() => act(() => read(c))}
                ><span class="chapter-title"
                  ><b>{c.title}</b><span class="tag"
                    >{c.read ? t('m_9b9a8d05a7ec', $locale) : t('m_1b9f384c1436', $locale)}</span
                  ></span
                ><small>{c.pageIds.length} {t('m_bfa062de040f', $locale)}</small></button
              >
              <div class="chapter-actions">
                <button
                  class="primary"
                  disabled={!c.pageIds.length}
                  title={tr(
                    !c.pageIds.length ? 'This chapter has no pages' : 'Read from the first page',
                    $locale,
                  )}
                  onclick={() => act(() => read(c))}>{t('m_9b9a8d05a7ec', $locale)}</button
                >{#if !compact.current}<button
                    disabled={!c.pageIds.length}
                    title={tr(
                      !c.pageIds.length ? 'This chapter has no pages' : 'Edit from the first page',
                      $locale,
                    )}
                    onclick={() => act(() => read(c, 'Editor'))}
                    >{t('m_464c4ffd019e', $locale)}</button
                  ><button
                    class="translate-chapter"
                    disabled={!c.pageIds.length ||
                      !readiness.ready('translate') ||
                      submitting.includes(book.path + '/' + c.id) ||
                      submitting.includes(book.path + '/all')}
                    title={tr(
                      !c.pageIds.length
                        ? 'This chapter has no pages'
                        : readiness.reason('translate') || 'Choose how to translate this chapter',
                      $locale,
                    )}
                    onclick={() => book && act(() => translate(book!, c))}
                    ><Languages size={16} />{tr('Translate chapter', $locale)}</button
                  >{/if}<button
                  title={t('m_c78795caa363', $locale)}
                  aria-label={tr(`Options for ${c.title}`, $locale)}
                  aria-haspopup="menu"
                  onclick={(e) => showChapterMenu(c, e)}>•••</button
                >
              </div>
            </article>{/each}{#if !book.chapters.length}<div class="empty">
              <h3>{t('m_73f8f43962ba', $locale)}</h3>
              {#if !hosted}<button
                  title={t('m_55f262372707', $locale)}
                  onclick={() => act(openOrganizer)}>{t('m_cb36926a2fc6', $locale)}</button
                >{/if}
            </div>{/if}
        </section>
      </main>
    {:else if screen === 'Reader' && reading && book}{#key `${book.id}/${chapterId}`}<Workspace
          bind:this={workspace}
          initial={reading}
          initialPageId={workspaceInitialPageId}
          omittedPageIds={book.omittedPageIds}
          {jobs}
          {jobsHeld}
          onjobcontrol={async (id, action) => {
            const result = await call('job_control', { id, action });
            applyJobs(result.state);
            if (result.outcome.errors.length)
              throw uiJoin(result.outcome.errors.map((e) => e.message));
          }}
          bind:mode={workspaceMode}
          bind:collapsed={readerCollapsed}
          {fullscreen}
          blocked={!!modal}
          onmodechange={workspaceModeChanged}
          onfullscreen={() => act(toggleFullscreen)}
          onexitfullscreen={() => act(() => setFullscreen(false))}
          ontheme={() => act(switchTheme)}
          onsettings={translationSettings}
          onglossary={() => act(openGlossary)}
          settings={readerSettings}
          {fonts}
          {chapterId}
          onexport={() => showExport(activeChapter || null)}
          onback={() => act(() => leave('Book'))}
          onnext={() => {
            if (nextChapter) act(() => read(nextChapter!));
          }}
          hasNext={!!nextChapter}
          oncomplete={() => {
            if (activeChapter && !activeChapter.read) act(() => complete(activeChapter!, true));
          }}
        />{/key}
    {:else if screen === 'Jobs'}<JobsView
        onedit={editJob}
        {openingJob}
        {historyMore}
        {loadingHistory}
        onhistory={() => act(loadHistory)}
        {jobs}
        {settings}
        onsettings={translationSettings}
        held={jobsHeld}
        recovering={jobsRecovering}
        errors={jobsErrors}
        oncontrolall={async (action) => {
          const result = await call('jobs_control_all', { action });
          applyJobs(result.state);
          return result.outcome;
        }}
        oncontrol={async (id, action) => {
          const result = await call('job_control', { id, action });
          applyJobs(result.state);
          if (result.outcome.errors.length)
            throw uiJoin(result.outcome.errors.map((e) => e.message));
        }}
      />{/if}
    {#if menu || chapterMenu}<button
        class="menu-dismiss"
        aria-label={t('m_4b613a130af7', $locale)}
        onclick={dismissMenu}
      ></button>
      <div
        class="book-menu"
        role="menu"
        aria-label={tr(`Options for ${chapterMenu?.title || menu?.metadata.title}`, $locale)}
        tabindex="-1"
        bind:this={menuElement}
        use:watchMenu
        style:left={`${Math.max(8, menuX)}px`}
        style:top={`${Math.max(8, menuY)}px`}
        onkeydown={menuKey}
      >
        {#each menuItems.filter((item) => !hosted || ['mark', 'translate', 'edit'].includes(item.action)) as item (item.action)}<button
            role="menuitem"
            title={tr(item.title || item.label, $locale)}
            disabled={item.disabled}
            class:danger={item.action === 'delete'}
            onclick={() => act(() => menuAction(item.action))}>{tr(item.label, $locale)}</button
          >{/each}
        {#if (chapterMenu?.pageIds.length || menu?.pages) && readiness.hint('translate')}<div
            class="menu-help"
            role="presentation"
          >
            <p role="note">{tr(readiness.hint('translate'), $locale)}</p>
            {#if hosted}<p>{t('host.configurePC', $locale)}</p>{:else}
              <button
                role="menuitem"
                title={t('m_d6e6b781138f', $locale)}
                onclick={() => {
                  dismissMenu();
                  translationSettings();
                }}>{t('m_5810fd24fe86', $locale)}</button
              >{/if}
          </div>{/if}
      </div>{/if}
    {#if libraryFiltersOpen && compact.current}<Modal
        title={t('mobile.filters', $locale)}
        onclose={() => (libraryFiltersOpen = false)}
        className="mobile-library-filters"
        ><div class="mobile-view-choice">
          <span>{t('mobile.view', $locale)}</span>{@render libraryViews()}
        </div>
        <div class="mobile-filter-choices">{@render libraryChoices()}</div>
        {#snippet footer()}<button
            onclick={() => {
              language = '';
              tag = '';
              completion = '';
            }}>{t('mobile.clearFilters', $locale)}</button
          ><span class="spacer"></span><button
            class="primary"
            onclick={() => (libraryFiltersOpen = false)}>{t('mobile.done', $locale)}</button
          >{/snippet}</Modal
      >{/if}
    {#if modal === 'translation' && translationTarget}{#key `${translationTarget.book.path}/${translationTarget.chapterId ?? 'all'}`}<TranslationBatchDialog
          book={translationTarget.book}
          onbook={glossarySaved}
          chapterId={translationTarget.chapterId}
          {settings}
          {jobs}
          onclose={() => {
            modal = '';
            translationTarget = null;
          }}
          onsettings={() => {
            translationTarget = null;
            translationSettings();
          }}
          onworking={(working) => {
            const scope =
              translationTarget?.book.path + '/' + (translationTarget?.chapterId ?? 'all');
            submitting = working ? [...submitting, scope] : submitting.filter((s) => s !== scope);
            if (!working && modal !== 'translation') translationTarget = null;
          }}
          onsubmitted={(result) => {
            notify('info', batchFeedback(result));
            modal = '';
          }}
        />{/key}{:else if modal === 'glossary' && modalBook}<GlossaryModal
        book={modalBook}
        {settings}
        onclose={() => {
          if (modal === 'glossary') modal = '';
        }}
        onsaved={glossarySaved}
      />{:else if modal === 'create'}<CreateBook
        scale={displayedSettings.appearance[displayedSettings.appearance.active].scale}
        onclose={() => (modal = '')}
        oncreated={(b) => act(() => imported(b))}
      />{:else if modal === 'organizer' && modalBook}<Organizer
        scale={displayedSettings.appearance[displayedSettings.appearance.active].scale}
        book={modalBook}
        onclose={() => (modal = '')}
        onapply={(b) => act(() => updatedBook(b))}
      />{:else if modal === 'hosting' && native}<HostDialog
        onclose={() => (modal = '')}
        onstatus={(s) => (hostingRunning = s.running)}
      />{:else if hosted && (modal === 'settings' || modal === 'book-settings')}<WebPreferences
        value={settings}
        onclose={() => (modal = '')}
        onapply={preferences}
      />{:else if modal === 'settings' || modal === 'book-settings'}<UnifiedSettings
        initialTab={settingsTab}
        onpreview={(a) => (appearancePreview = a)}
        value={settings}
        {models}
        {catalog}
        {fonts}
        book={modal === 'book-settings' && modalBook ? modalBook : undefined}
        onbook={(b) => {
          if (book?.path === b.path) book = b;
          modalBook = b;
          act(refresh);
        }}
        onclose={() => {
          modal = '';
          appearancePreview = null;
          applyTheme(settings.appearance);
        }}
        onapply={preferences}
        onrelocate={relocateLibrary}
      />{:else if modal === 'export' && modalBook}<ExportDialog
        path={modalBook.path}
        title={exportChapter?.title || modalBook.metadata.title}
        chapterId={exportChapter?.id || null}
        onclose={() => (modal = '')}
        oncomplete={() => {
          modal = '';
          notify('success', 'Export complete.');
        }}
      />{:else if modal === 'metadata' && modalBook && editMetadata}<Modal
        title={t('m_a4ce10f7b615', $locale)}
        wide
        onclose={() => {
          if (!busy) modal = '';
        }}
      >
        <fieldset class="organizer-controls" disabled={busy}>
          <BookDetailsFields
            value={editMetadata}
            bind:cover={editCover}
            path={modalBook.path}
            pageId={modalBook.chapters.find((c) => c.pageIds.length)?.pageIds[0]}
          />
        </fieldset>
        {#if metadataError}<p class="dialog-error" role="alert">
            {tr(metadataError, $locale)}
          </p>{/if}
        {#snippet footer()}<button disabled={busy} onclick={() => (modal = '')}
            >{t('m_19766ed6ccb2', $locale)}</button
          >
          <button
            class="primary"
            disabled={busy || !editMetadata?.title.trim()}
            onclick={() => void applyMetadata()}>{t('m_31e392d1c037', $locale)}</button
          >
        {/snippet}
      </Modal>{/if}
    {#if busy}<div class="busy">{t('m_5474eef8d0f1', $locale)}</div>{/if}
  </div>{:else}<div class="startup">U-Manga</div>{/if}
<NotificationHost
  controller={notifications}
  reader={screen === 'Reader' && workspaceMode === 'Reader'}
/>
