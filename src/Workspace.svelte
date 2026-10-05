<script lang="ts">
  import { refreshQueue } from './refresh-queue';
  import { useNotifications } from './notification-context';
  import type { NoticeSeverity } from './notifications';
  const notifications = useNotifications();
  const coverageReason = `workspace-${randomUuid()}`;
  const call = scopedCommands(captureNoticeOwner);
  function captureNoticeOwner() {
    const id = page?.id,
      path = project?.path,
      generation = editor.generation;
    return () =>
      !disposed && page?.id === id && project?.path === path && editor.generation === generation;
  }
  function notify(severity: NoticeSeverity, message: UiText) {
    if (disposed) return;
    const dialog = dialogIdentity();
    if (dialog && (severity === 'error' || severity === 'warning')) {
      dialogErrorOwner = dialog;
      dialogError = message;
    } else notifications.publish(severity, message);
  }
  onDestroy(() => {
    disposed = true;
    notifications.cover(coverageReason, false);
  });
  import { randomUuid } from './browser-id';
  import { uiError, type UiText, t, tr, locale } from './i18n';
  import type { JobAction } from './commands';
  import { EditorSession } from './editor-session.svelte';
  import { AsyncOwner } from './async-owner';
  import { useCompactLayout } from './mobile-layout.svelte';
  import { visibleViewport, observeViewport, placeOverlay } from './viewport';
  import { modalIdentity, modalStack, registerModal } from './modal-stack';
  import './mobile-workspace.css';
  const compact = useCompactLayout();
  const editor = new EditorSession();
  const navigation = new AsyncOwner();
  import { listenContent } from './content-events';
  import PageViewSelector from './PageViewSelector.svelte';
  import ComboBox from './ComboBox.svelte';
  import ToggleSwitch from './ToggleSwitch.svelte';
  import { onDestroy, onMount, untrack } from 'svelte';
  import { tick } from 'svelte';
  import { open } from '@tauri-apps/plugin-dialog';
  import { submitJobs, submitPreparation, submittedJobs } from './job-events';
  import { actionReadiness, jobsReadiness } from './action-readiness.svelte';
  import RequirementHint from './RequirementHint.svelte';
  import ReaderJobBadge from './ReaderJobBadge.svelte';
  import { latestPageJob, mergePageEdits } from './editor-state';
  import { jobIndex } from './job-index';
  import {
    scopedDraft,
    remainingDraft,
    acknowledgeDraft,
    hasDraftChanges,
    changedRegions,
    type EditScope,
  } from './editor-drafts';
  import EditorProgress from './EditorProgress.svelte';
  import { RegionOperations } from './region-operation.svelte';
  import { acceptsRegionResult } from './region-result';
  const regionOps = new RegionOperations();
  import Modal from './Modal.svelte';
  import {
    BookOpen,
    BookA,
    ArrowLeft,
    Plus,
    ChevronLeft,
    ChevronRight,
    Undo2,
    Redo2,
    ScanText,
    Languages,
    Check,
    ImagePlus,
    Trash2,
    MoreHorizontal,
    PanelBottom,
    PenLine,
  } from 'lucide-svelte';
  import {
    handles,
    beginGesture,
    updateGesture,
    imagePoint,
    moveBox,
    setRegionBox,
    geometry,
    applyGeometryEdit,
    type Gesture,
    type GeometryEdit,
  } from './region-editing';
  import type { Settings, Project, Page, Job, Region } from './types';
  import { pageFilter } from './appearance';
  import { scopedCommands, hosted, native, imageUrl } from './bridge';
  import PageView from './PageView.svelte';
  import PageStrip from './PageStrip.svelte';
  import ReaderControls from './ReaderControls.svelte';
  let {
    initial,
    omittedPageIds,
    settings,
    fonts,
    chapterId,
    onback,
    onnext,
    oncomplete,
    onexport,
    hasNext,
    mode = $bindable('Reader'),
    collapsed = $bindable(false),
    fullscreen = false,
    blocked = false,
    onmodechange,
    onfullscreen,
    onexitfullscreen,
    ontheme,
    onsettings,
    onglossary,
    jobs,
    jobsHeld,
    onjobcontrol,
  }: {
    initial: Project;
    omittedPageIds?: string[];
    settings: Settings;
    fonts: string[];
    chapterId: string;
    onback: () => void;
    onnext: () => void;
    oncomplete: () => void;
    onexport: () => void;
    hasNext: boolean;
    mode?: string;
    collapsed?: boolean;
    fullscreen?: boolean;
    blocked?: boolean;
    onmodechange: (mode: string) => Promise<void>;
    onfullscreen: () => void;
    onexitfullscreen: () => void;
    ontheme: () => void;
    onsettings: () => void;
    onglossary: () => void;
    jobs: Job[];
    jobsHeld: boolean;
    onjobcontrol: (id: string, action: JobAction) => Promise<void>;
  } = $props();
  let project = $state<Project | null>(untrack(() => structuredClone($state.snapshot(initial))));
  let omittedPages = $derived(new Set(omittedPageIds ?? project?.omittedPageIds));
  let disposed = false;
  let completed = $state(false);
  let index = $state(0);
  let showPages = $state(false);
  let readerCanvas = $state<HTMLDivElement>();
  let positioningReader = false;
  let translated = $state(true);
  let continuous = $state(untrack(() => settings.reader.layout === 'continuous'));
  let zoom = $state(untrack(() => settings.reader.zoom));
  let busy = $state(false);
  let dialogError = $state<UiText>('');
  let dialogErrorOwner = $state('');
  $effect(() => notifications.cover(coverageReason, busy));
  let readerTranslate = $state(false);
  let editorSrc = $state('');
  let imageRequest = $state(0);
  let webEditorImage = $state<HTMLImageElement>();
  function webImageReady() {
    if (
      !disposed &&
      imagePage?.id === page?.id &&
      webEditorImage?.complete &&
      webEditorImage.naturalWidth > 0 &&
      editorSrc &&
      webEditorImage.src === new URL(editorSrc, document.baseURI).href
    )
      imageReady = true;
  }
  let imageReady = $state(false);
  let imagePage = $state<{
    path: string;
    id: string;
    generation: number;
    revision: number;
    translated: boolean;
  } | null>(null);
  let discarding = $state(false);
  let tool = $state('select');
  let geometryExpanded = $state(true);
  let regionAction = $state<'draw' | 'move'>('move');
  let gesture = $state<Gesture | null>(null);
  let geometryUndo = $state<GeometryEdit[]>([]);
  let geometryRedo = $state<GeometryEdit[]>([]);
  const deferredPages = new Map<string, Page>();
  let canvas = $state<HTMLDivElement>();
  let saving = $state(false);
  let backgroundPrompt = $state<{
    pageId: string;
    path: string;
    generation: number;
    background: string | null;
  } | null>(null);
  let deletePrompt = $state<string | null>(null);
  let saveInFlight: Promise<Page> | null = null;
  let leavePrompt = $state(false);
  let resolveLeave: ((value: boolean) => void) | null = null;
  const requested = new Set<string>();
  const submitting = new Set<string>();
  let submittingCount = $state(0);
  let savedPage = $derived(project?.pages[index] || null);
  let page = $derived(editor.draft?.id === savedPage?.id ? editor.draft : savedPage);
  const imageOwned = $derived(
    imageReady &&
      !!imagePage &&
      !disposed &&
      imagePage.path === project?.path &&
      imagePage.id === page?.id &&
      imagePage.generation === editor.generation &&
      imagePage.revision === savedPage?.revision &&
      imagePage.translated === translated,
  );
  let pageJob = $derived(latestPageJob(jobs, project?.path, page?.id));
  const translationReadiness = actionReadiness(() => ({
    settings,
    path: project?.path,
    actions: ['translate'],
  }));
  const regionReadiness = actionReadiness(() => ({
    settings,
    path: project?.path,
    actions: mode === 'Editor' && !hosted ? ['region_read', 'region_translate'] : [],
  }));
  const readiness = actionReadiness(() => ({
    settings,
    path: project?.path,
    page: mode === 'Editor' ? page : undefined,
    base: mode === 'Editor' ? editor.baseline || savedPage : undefined,
    actions: mode === 'Editor' ? ['prepare', 'edit'] : [],
  }));
  const jobAvailability = jobsReadiness(
    () => (pageJob ? [pageJob] : []),
    () => settings,
  );
  let preparationPrompt = $state<{
    id: string;
    revision: number;
    path: string;
    action: 'prepare' | 'translate';
  } | null>(null);
  let preparing = $state(false);
  $effect(() => {
    if (jobsHeld && regionOps.busy) void regionOps.cancel();
  });
  let chapterPageIds = $derived(new Set(project?.pages.map((p) => p.id) || []));
  let readerPending = $derived(jobIndex(jobs).pending(project?.path, chapterPageIds, page?.id));
  $effect(() => {
    if (readerTranslate && translationReadiness.fresh && !translationReadiness.ready('translate')) {
      readerTranslate = false;
      notify('warning', translationReadiness.reason('translate'));
    }
  });
  let cleanedPreview = $derived(
    !!(
      (native || hosted) &&
      translated &&
      savedPage?.cleanup &&
      pageJob?.pageRevision === savedPage.revision &&
      !pageJob?.steps.some((s) => s.stage === 'saving' && s.status === 'complete') &&
      pageJob?.steps.some(
        (s) => s.stage === 'cleaning' && ['complete', 'warning'].includes(s.status),
      )
    ),
  );
  let pageBusy = $derived(
    !!pageJob && (['queued', 'running', 'paused'].includes(pageJob.status) || pageJob.stopping),
  );
  let region = $derived(page?.regions[editor.selectedRegion] || null);
  let dirtyRegionIds = $derived(
    new Set(editor.baseline && page ? changedRegions(editor.baseline, page) : []),
  );
  let selectedDirty = $derived(!!region && dirtyRegionIds.has(region.id));
  // Saved history has one confirmation transaction, including missing regions.
  const confirmationScope = $derived<EditScope | undefined>(
    editor.pendingHistory
      ? editor.pendingHistory.commit.scope
      : region
        ? { regionId: region.id }
        : undefined,
  );
  const confirmationDirty = $derived(editor.pendingHistory ? editor.dirty : selectedDirty);
  const confirmationAll = $derived(!!editor.pendingHistory && !confirmationScope);
  const selectedReadiness = actionReadiness(() => ({
    settings,
    path: project?.path,
    base: editor.baseline || savedPage,
    page:
      page && confirmationScope && editor.baseline
        ? scopedDraft($state.snapshot(editor.baseline), $state.snapshot(page), confirmationScope)
        : page,
    actions: mode === 'Editor' && (region || editor.pendingHistory) ? ['edit'] : [],
  }));
  const backgroundReadiness = actionReadiness(() => ({
    settings,
    path: project?.path,
    base: savedPage,
    page:
      backgroundPrompt && savedPage
        ? { ...$state.snapshot(savedPage), background: backgroundPrompt.background }
        : undefined,
    actions: backgroundPrompt ? ['edit'] : [],
  }));
  let theme = $derived(settings.appearance[settings.appearance.active]);
  let availableWidth = $state(typeof window !== 'undefined' ? window.innerWidth : 1100);
  let mobileSheet = $state(false);
  let editorMore = $state(false);
  let editorThumbnails = $state(false);
  let thumbnailsMounted = $state(false);
  const thumbnailsShown = $derived(
    mode === 'Editor' ? !compact.current || editorThumbnails : showPages && !collapsed,
  );
  $effect(() => {
    if (thumbnailsShown) thumbnailsMounted = true;
  });
  let moreTrigger = $state<HTMLButtonElement>();
  let morePanel = $state<HTMLDivElement>();
  let inspector = $state<HTMLElement>();
  let moreBounds = $state({ left: 0, top: 0, width: 320, maxHeight: 400, above: false });
  let sheetBounds = $state({ left: 0, top: 0, width: 320, height: 500 });
  const sheetOwner = modalIdentity();
  const sheetActive = $derived(compact.current && mobileSheet && mode === 'Editor');
  const sheetTop = $derived($modalStack.at(-1) === sheetOwner);
  const sheetRank = $derived(Math.max(0, $modalStack.indexOf(sheetOwner)));
  function dialogIdentity() {
    return leavePrompt
      ? 'leave'
      : backgroundPrompt
        ? 'background'
        : deletePrompt
          ? 'delete'
          : preparationPrompt
            ? 'preparation'
            : sheetActive
              ? 'sheet'
              : '';
  }
  $effect(() => {
    const owner = dialogIdentity();
    if (owner !== dialogErrorOwner) {
      dialogError = '';
      dialogErrorOwner = owner;
    }
  });
  function positionWorkspaceOverlays() {
    if (moreTrigger && morePanel)
      moreBounds = placeOverlay(moreTrigger.getBoundingClientRect(), 340, morePanel.scrollHeight);
    const viewport = visibleViewport();
    const height = Math.min(660, viewport.height * 0.86);
    sheetBounds = {
      left: viewport.left,
      top: viewport.bottom - height,
      width: viewport.width,
      height,
    };
  }
  async function toggleEditorMore() {
    editorMore = !editorMore;
    if (editorMore) {
      await tick();
      positionWorkspaceOverlays();
      morePanel?.focus({ preventScroll: true });
    }
  }
  function editorOption(action: () => void) {
    editorMore = false;
    action();
  }
  function editorMoreKey(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      editorMore = false;
      moreTrigger?.focus({ preventScroll: true });
    }
  }
  function sheetKey(event: KeyboardEvent) {
    if (!sheetActive || !sheetTop || event.defaultPrevented) return;
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      mobileSheet = false;
    } else if (event.key === 'Tab' && inspector) {
      const items = Array.from(
        inspector.querySelectorAll<HTMLElement>(
          'button:not(:disabled),input:not(:disabled),textarea:not(:disabled),[tabindex="0"]',
        ),
      ).filter((element) => element.offsetParent !== null);
      const first = items[0],
        last = items.at(-1);
      if (
        event.shiftKey &&
        (document.activeElement === first || document.activeElement === inspector)
      ) {
        event.preventDefault();
        last?.focus();
      } else if (
        !event.shiftKey &&
        (document.activeElement === last || document.activeElement === inspector)
      ) {
        event.preventDefault();
        first?.focus();
      }
    }
  }
  $effect(() => {
    if (!sheetActive) return;
    return untrack(() => {
      const before = document.activeElement as HTMLElement | null;
      const remove = registerModal(sheetOwner);
      positionWorkspaceOverlays();
      void tick().then(() => {
        if (sheetActive && sheetTop) inspector?.focus({ preventScroll: true });
      });
      return () => {
        remove();
        void tick().then(() => {
          if (
            before?.isConnected &&
            before !== document.body &&
            !before.closest('[inert]') &&
            !before.matches(':disabled') &&
            before.getClientRects().length
          )
            before.focus({ preventScroll: true });
          else if (!disposed && compact.current && mode === 'Editor')
            moreTrigger?.focus({ preventScroll: true });
        });
      };
    });
  });
  let remoteSaved = $state<Page | null>(null);
  function reviewRemote() {
    if (!remoteSaved || !editor.baseline || !editor.draft) return;
    try {
      editor.draft = mergePageEdits(
        $state.snapshot(editor.baseline),
        $state.snapshot(editor.draft),
        $state.snapshot(remoteSaved),
      );
      editor.baseline = structuredClone($state.snapshot(remoteSaved));
      remoteSaved = null;
    } catch (e) {
      fail(e);
    }
  }
  function fitSurface(node: HTMLElement) {
    let frame = 0;
    const observer = new ResizeObserver((entries) => {
      const next = entries[0]?.contentRect.width;
      if (!next || !hosted || Math.abs(next - availableWidth) < 1) return;
      cancelAnimationFrame(frame);
      // Applying image dimensions inside a ResizeObserver callback creates a
      // scrollbar/width feedback loop in WebKit. Commit on the next frame.
      frame = requestAnimationFrame(() => {
        if (!node.isConnected) return;
        const scroller = mode === 'Reader' ? readerCanvas : node;
        const el =
          mode === 'Reader'
            ? readerCanvas?.querySelector<HTMLElement>(`[data-page="${page?.id}"]`)
            : node.querySelector<HTMLElement>('.editor-paper');
        const fraction =
          el && scroller
            ? (scroller.getBoundingClientRect().top - el.getBoundingClientRect().top) /
              el.clientHeight
            : null;
        availableWidth = next;
        if (fraction !== null && el && scroller)
          void tick().then(() => {
            if (el.isConnected && scroller.isConnected)
              scroller.scrollTop +=
                el.getBoundingClientRect().top -
                scroller.getBoundingClientRect().top +
                fraction * el.clientHeight;
          });
      });
    });
    observer.observe(node);
    return {
      destroy: () => {
        observer.disconnect();
        cancelAnimationFrame(frame);
      },
    };
  }
  // ResizeObserver.contentRect already excludes the canvas padding.
  let width = $derived(Math.round(((hosted ? Math.max(100, availableWidth) : 1100) * zoom) / 100));
  function fail(e: unknown, current = captureNoticeOwner(), dialog = dialogIdentity()) {
    if (!current()) return;
    if (dialog && dialog === dialogIdentity()) {
      dialogErrorOwner = dialog;
      dialogError = uiError(e);
    } else if (!dialog) notify('error', uiError(e));
    busy = false;
  }
  async function act(f: () => Promise<unknown>) {
    let current = captureNoticeOwner();
    const beforeNavigation = navigation.capture(),
      dialog = dialogIdentity();
    try {
      const pending = f();
      // Navigation starts before its first await. Its own image failure belongs
      // to the requested page; a later navigation still invalidates this owner.
      if (!beforeNavigation()) {
        const ownedNavigation = navigation.capture();
        current = () => !disposed && ownedNavigation();
      }
      await pending;
    } catch (e) {
      fail(e, current, dialog);
    }
  }
  function reachedEnd() {
    if (!completed) {
      completed = true;
      oncomplete();
    }
  }
  function endVisible(el: HTMLElement) {
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting && e.intersectionRatio >= 0.5)) reachedEnd();
      },
      // Use the scrolling viewport explicitly; fractional WebView2 zoom can keep
      // a bottom-aligned footer just below a 100% intersection threshold.
      { root: el.closest('.reader-canvas'), threshold: 0.5 },
    );
    observer.observe(el);
    return {
      destroy() {
        observer.disconnect();
      },
    };
  }
  async function navigate(n: number) {
    const current = navigation.begin();
    if (!(await beforeLeave()) || !current()) return;
    selectPage(n);
    await editorImage();
    await tick();
    if (continuous)
      readerCanvas?.querySelector(`[data-page="${page?.id}"]`)?.scrollIntoView({ block: 'start' });
    else if (readerCanvas) readerCanvas.scrollTop = 0;
    if (readerTranslate) await prefetch();
  }
  function selectPage(n: number) {
    mobileSheet = false;
    editorMore = false;
    remoteSaved = null;
    if (!project?.pages.length || n < 0) return;
    if (index !== Math.min(project.pages.length - 1, n)) regionOps.state = null;
    invalidateEditorImage();
    index = Math.min(project.pages.length - 1, n);
    editor.enter(project.pages[index].id);
    cancelGesture();
    geometryUndo = [];
    geometryRedo = [];
    if (invalidatedPages.has(project.pages[index].id)) void refreshPage(project.pages[index].id);
  }
  async function selectView(value: boolean) {
    if (value === translated) return;
    cancelGesture();
    translated = value;
    await editorImage();
  }
  function invalidateEditorImage(preserve = false) {
    imageRequest++;
    if (!preserve) editorSrc = '';
    imageReady = false;
    imagePage = null;
  }
  async function editorImage() {
    if (!page || !project || disposed) return;
    editor.enter(page.id);
    invalidateEditorImage(hosted && imagePage?.id === page.id);
    const request = imageRequest;
    const target = {
      path: project.path,
      id: page.id,
      generation: editor.generation,
      revision: savedPage!.revision,
      translated,
    };
    imagePage = target;
    const current = () =>
      !disposed &&
      request === imageRequest &&
      project?.path === target.path &&
      page?.id === target.id &&
      editor.generation === target.generation &&
      savedPage?.revision === target.revision &&
      translated === target.translated;
    try {
      const src =
        cleanedPreview && savedPage?.cleanup
          ? savedPage.cleanup.path
          : await call('page_image', {
              path: target.path,
              pageId: target.id,
              translated: target.translated,
            });
      if (current()) {
        editorSrc = imageUrl(src);
        await tick();
        if (current() && hosted) webImageReady(); // The same cached URL need not fire load again.
      }
    } catch (error) {
      if (current()) throw error;
    }
  }
  async function enqueue(ids: string[], book = project, priority = false) {
    const current = captureNoticeOwner();
    if (!book) return;
    const fresh = ids.filter((id) => !submitting.has(`${book.id}/${id}`));
    if (!fresh.length) return;
    for (const id of fresh) submitting.add(`${book.id}/${id}`);
    submittingCount = submitting.size;
    try {
      const added = await submitJobs(book.path, fresh, priority, call);
      if (current() && added.some((j) => j.status === 'paused'))
        notify('info', 'Jobs added as paused. Use Start all in Jobs.');
      for (const id of fresh) requested.add(`${book.id}/${id}`);
    } finally {
      for (const id of fresh) submitting.delete(`${book.id}/${id}`);
      submittingCount = submitting.size;
    }
  }
  async function prefetch() {
    if (
      mode !== 'Reader' ||
      !project ||
      !readerTranslate ||
      !translationReadiness.ready('translate')
    )
      return;
    const pages = project.pages
      .slice(index, index + 3)
      .filter(
        (p) =>
          !omittedPages.has(p.id) &&
          ['new', 'prepared'].includes(p.status) &&
          !requested.has(`${project!.id}/${p.id}`),
      );
    if (pages.length)
      await enqueue(
        pages.map((p) => p.id),
        project,
        true,
      );
  }
  $effect(() => {
    const enabled = mode === 'Reader' && readerTranslate && translationReadiness.ready('translate');
    omittedPages;
    if (enabled) untrack(() => act(prefetch));
  });
  function changed(resetRedo: unknown = true) {
    if (resetRedo !== false) {
      editor.redo = [];
      editor.pendingHistory = null;
    }
    editor.dirty = !!editor.baseline && !!page && hasDraftChanges(editor.baseline, page);
    editor.epoch++;
  }
  function acceptSaved(saved: Page, pending: Page | null, scope?: EditScope, baseline = saved) {
    if (!project || page?.id !== saved.id) return;
    const selected = region?.id;
    project.pages[index] = saved;
    editor.dirty = !!pending && hasDraftChanges(baseline, pending);
    editor.baseline = editor.dirty ? structuredClone(baseline) : null;
    editor.draft = editor.dirty ? pending : null;
    if (scope && 'regionId' in scope) {
      geometryUndo = geometryUndo.filter((e) => e.regionId !== scope.regionId);
      geometryRedo = geometryRedo.filter((e) => e.regionId !== scope.regionId);
    } else if (!scope) {
      geometryUndo = [];
      geometryRedo = [];
    }
    if (selected)
      editor.selectedRegion = Math.max(
        0,
        page!.regions.findIndex((r) => r.id === selected),
      );
    editor.epoch++;
  }
  async function saveDraft(snapshot: Page, scope?: EditScope) {
    cancelGesture();
    if (saving || !page || !project) return;
    if (
      editor.pendingHistory &&
      JSON.stringify(scope) !== JSON.stringify(editor.pendingHistory.commit.scope)
    )
      throw Error('Confirm or discard the pending Undo/Redo change first.');
    saving = true;
    const base = structuredClone($state.snapshot(editor.baseline || savedPage!));
    const sent = scope ? scopedDraft(base, snapshot, scope) : snapshot;
    const owns = editor.capture();
    const path = project.path;
    try {
      saveInFlight = call('edit_page', { path, page: sent, base, expected: base.revision });
      const saved = await saveInFlight;
      if (!owns() || project?.path !== path || savedPage?.id !== saved.id) return;
      if (
        editor.pendingHistory &&
        (!scope || JSON.stringify(editor.pendingHistory.commit.scope) === JSON.stringify(scope))
      ) {
        const history = editor.pendingHistory;
        const from = history.back ? editor.undo : editor.redo;
        const to = history.back ? editor.redo : editor.undo;
        from.pop();
        to.push(history.commit);
      } else {
        editor.undo.push({ before: base, after: sent, scope });
        editor.redo = [];
      }
      editor.pendingHistory = null;
      const acknowledged = acknowledgeDraft(
        base,
        snapshot,
        saved,
        $state.snapshot(savedPage),
        scope,
      );
      acceptSaved(acknowledged.saved, acknowledged.pending, scope, acknowledged.baseline);
      if (acknowledged.warning) fail(acknowledged.warning);
      await editorImage();
    } finally {
      saving = false;
      saveInFlight = null;
    }
  }
  async function confirmEdits(scope?: EditScope) {
    if (!editor.dirty || !page) return;
    await saveDraft(
      structuredClone($state.snapshot(page)),
      editor.pendingHistory ? editor.pendingHistory.commit.scope : scope,
    );
  }
  async function discardEdits(scope?: EditScope) {
    if (saving || discarding || !project || !page) return;
    if (hosted && remoteSaved) {
      editor.baseline = structuredClone($state.snapshot(remoteSaved));
      remoteSaved = null;
    }
    if (editor.pendingHistory) scope = editor.pendingHistory.commit.scope;
    const target = { path: project.path, id: page.id, operation: editor.generation };
    const base = structuredClone($state.snapshot(editor.baseline || savedPage!));
    const draft = structuredClone($state.snapshot(page));
    discarding = true;
    try {
      let latest = await call('page_get', { path: target.path, pageId: target.id });
      if (
        project?.path !== target.path ||
        page?.id !== target.id ||
        editor.generation !== target.operation
      )
        return;
      // A checkpoint event may have delivered a newer page while this read was in flight.
      if (project.pages[index].revision > latest.revision)
        latest = structuredClone($state.snapshot(project.pages[index]));
      const pending = scope ? remainingDraft(base, draft, latest, scope) : null;
      acceptSaved(latest, pending, scope);
      editor.pendingHistory = null;
      await editorImage();
    } finally {
      discarding = false;
    }
  }
  export async function beforeLeave(): Promise<boolean> {
    cancelGesture();
    await regionOps.cancel();
    if (discarding) return false;
    if (saving) {
      await saveInFlight;
      await tick();
    }
    if (!editor.dirty) return true;
    if (leavePrompt) return false;
    leavePrompt = true;
    return new Promise((resolve) => {
      resolveLeave = resolve;
    });
  }
  async function resolveEdits(action: 'confirm' | 'discard' | 'stay') {
    if (action === 'confirm') await confirmEdits();
    if (action === 'discard') await discardEdits();
    if (action !== 'stay' && editor.dirty) return;
    leavePrompt = false;
    resolveLeave?.(action !== 'stay');
    resolveLeave = null;
  }
  function editStart() {
    if (savedPage) editor.begin($state.snapshot(savedPage));
  }
  async function preparePage() {
    if (!page || !project || !readiness.ready('prepare') || preparing) return;
    if (!(await beforeLeave()) || !page) return;
    const target = {
      id: page.id,
      revision: page.revision,
      path: project!.path,
      action: 'prepare' as const,
    };
    if (page.regions.length || page.rendered || page.cleanup || page.background)
      preparationPrompt = target;
    else await submitPrepare(target, false);
  }
  async function submitPrepare(
    target: { id: string; revision: number; path: string; action: 'prepare' | 'translate' },
    replace: boolean,
  ) {
    if (!project) return;
    const current = captureNoticeOwner();
    regionOps.state = null;
    preparing = true;
    try {
      const added =
        target.action === 'prepare'
          ? await submitPreparation(target.path, target.id, target.revision, replace, call)
          : submittedJobs(
              await call('restart_enqueue', {
                path: target.path,
                pageId: target.id,
                expected: target.revision,
                replace,
              }),
            );
      if (!current()) return;
      notify(
        'info',
        added.some((j) => j.status === 'paused')
          ? 'Job added as paused. Use Start all in Jobs.'
          : added.length
            ? 'Page job added.'
            : 'This page already has an operation; wait for it to finish',
      );
      preparationPrompt = null;
    } finally {
      preparing = false;
    }
  }
  async function history(back: boolean) {
    cancelGesture();
    const local = back ? geometryUndo : geometryRedo;
    if (local.length && page) {
      editStart();
      const entry = $state.snapshot(local.pop()!);
      const draft = $state.snapshot(page!);
      if (applyGeometryEdit(draft, entry, back)) {
        editor.draft = draft;
        (back ? geometryRedo : geometryUndo).push(entry);
        editor.selectedRegion = Math.max(
          0,
          page!.regions.findIndex((r) => r.id === entry.regionId),
        );
        changed();
      } else
        notify('warning', 'This region was removed. Select a current region to continue editing.');
      return;
    }
    if (!(await beforeLeave())) return;
    if (!page || !project) return;
    const from = back ? editor.undo : editor.redo;
    const previous = from.at(-1);
    if (!previous) return;
    const commit = structuredClone($state.snapshot(previous));
    const restored = mergePageEdits(
      back ? commit.after : commit.before,
      back ? commit.before : commit.after,
      $state.snapshot(page),
    );
    editor.pendingHistory = { back, commit };
    if (commit.scope && 'background' in commit.scope) {
      backgroundPrompt = {
        pageId: page.id,
        path: project.path,
        generation: editor.generation,
        background: restored.background,
      };
    } else {
      const id = changedRegions($state.snapshot(page), restored)[0];
      editStart();
      editor.draft = restored;
      if (id)
        editor.selectedRegion = Math.max(
          0,
          restored.regions.findIndex((r) => r.id === id),
        );
      changed(false);
    }
  }
  function drawRegion() {
    if (hosted) return;
    cancelGesture();
    tool = 'edit';
    regionAction = 'draw';
  }
  function makeRegion(box: number[]): Region {
    return {
      id: randomUuid(),
      bbox: box,
      bubble: null,
      kind: 'manual',
      score: 1,
      source: '',
      target: '',
      direction: 'auto',
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
      review: null,
    };
  }
  function pointerPoint(e: PointerEvent): [number, number] {
    return imagePoint(
      [e.clientX, e.clientY],
      canvas!.getBoundingClientRect(),
      page!.width,
      page!.height,
    );
  }
  function boxFor(r: Region) {
    return gesture?.regionId === r.id ? gesture.box : r.bbox;
  }
  function clearGesture() {
    const previous = gesture ? $state.snapshot(gesture) : null;
    gesture = null;
    if (previous && canvas?.hasPointerCapture(previous.pointerId))
      canvas.releasePointerCapture(previous.pointerId);
    return previous;
  }
  function flushDeferred() {
    const updates = [...deferredPages.values()];
    deferredPages.clear();
    for (const updated of updates) {
      try {
        applyPageUpdate(updated);
      } catch (e) {
        fail(e);
      }
    }
  }
  function cancelGesture() {
    clearGesture();
    flushDeferred();
  }
  function down(e: PointerEvent) {
    if (
      !page ||
      !imageOwned ||
      saving ||
      discarding ||
      blocked ||
      tool !== 'edit' ||
      e.button !== 0 ||
      gesture
    )
      return;
    const target = e.target as HTMLElement;
    const id = target.closest<HTMLElement>('[data-region-id]')?.dataset.regionId;
    const selected = id ? page.regions.find((r) => r.id === id) || null : null;
    if (regionAction === 'move' && !selected) return;
    if (selected && regionAction === 'move') editor.selectedRegion = page.regions.indexOf(selected);
    if (regionAction === 'move')
      target.closest<HTMLButtonElement>('button')?.focus({ preventScroll: true });
    const handle = target.closest<HTMLElement>('[data-resize]')?.dataset.resize as
      Gesture['action'] | undefined;
    gesture = beginGesture(
      page,
      regionAction === 'draw' ? null : selected,
      regionAction === 'draw' ? 'draw' : handle || 'move',
      e.pointerId,
      pointerPoint(e),
      [e.clientX, e.clientY],
    );
    canvas!.setPointerCapture(e.pointerId);
    e.preventDefault();
  }
  function move(e: PointerEvent) {
    if (!gesture || e.pointerId !== gesture.pointerId || page?.id !== gesture.pageId) return;
    gesture = updateGesture(gesture, pointerPoint(e), [e.clientX, e.clientY]);
  }
  function recordGeometry(entry: GeometryEdit) {
    geometryUndo.push(entry);
    geometryRedo = [];
    changed();
  }
  function up(e: PointerEvent) {
    if (!gesture || e.pointerId !== gesture.pointerId) return;
    move(e);
    const completed = clearGesture()!;
    const refreshed = deferredPages.get(completed.pageId);
    const removed =
      completed.regionId &&
      refreshed &&
      !refreshed.regions.some((r) => r.id === completed.regionId);
    flushDeferred();
    if (removed) {
      notify('warning', 'This region was removed. Select a current region to continue editing.');
      return;
    }
    if (!page || page.id !== completed.pageId || !completed.meaningful) return;
    if (completed.action === 'draw') {
      if (completed.box[2] - completed.box[0] < 1 || completed.box[3] - completed.box[1] < 1)
        return;
      editStart();
      const added = makeRegion(completed.box);
      page!.regions.push(added);
      editor.selectedRegion = page!.regions.length - 1;
      recordGeometry({
        pageId: page!.id,
        regionId: added.id,
        before: null,
        after: geometry(added),
        created: structuredClone(added),
        index: editor.selectedRegion,
      });
      regionAction = 'move';
      void tick().then(() => {
        if (page?.id === completed.pageId)
          canvas
            ?.querySelector<HTMLButtonElement>(`[data-region-id="${added.id}"]`)
            ?.focus({ preventScroll: true });
      });
    } else {
      const current = page.regions.find((r) => r.id === completed.regionId);
      if (!current) {
        notify('warning', 'This region was removed. Select a current region to continue editing.');
        return;
      }
      updateBox(current.id, completed.box);
    }
  }
  function updateBox(id: string, box: number[]) {
    if (!page || !imageOwned || saving || discarding) return;
    const current = page.regions.find((r) => r.id === id);
    if (!current || current.bbox.every((n, i) => n === box[i])) return;
    const before = geometry($state.snapshot(current));
    editStart();
    const edited = page!.regions.find((r) => r.id === id)!;
    if (!setRegionBox(edited, box, page!.width, page!.height)) return;
    recordGeometry({
      pageId: page!.id,
      regionId: id,
      before,
      after: geometry($state.snapshot(edited)),
      index: page!.regions.indexOf(edited),
    });
  }
  function coordinate(i: number, value: number) {
    if (!region || !page) return;
    const box = [...region.bbox];
    box[i] = value;
    if (!box.every(Number.isFinite) || box[2] <= box[0] || box[3] <= box[1]) {
      notify('warning', 'Regions need a positive width and height');
      return;
    }
    updateBox(region.id, box);
  }
  function regionKey(e: KeyboardEvent, id: string) {
    if (
      !imageOwned ||
      saving ||
      discarding ||
      tool !== 'edit' ||
      regionAction !== 'move' ||
      !page ||
      !e.key.startsWith('Arrow') ||
      e.ctrlKey ||
      e.metaKey ||
      e.altKey
    )
      return;
    const r = page.regions.find((r) => r.id === id);
    if (!r) return;
    e.preventDefault();
    e.stopPropagation();
    const step = e.shiftKey ? 10 : 1;
    updateBox(
      id,
      moveBox(
        r.bbox,
        e.key === 'ArrowRight' ? step : e.key === 'ArrowLeft' ? -step : 0,
        e.key === 'ArrowDown' ? step : e.key === 'ArrowUp' ? -step : 0,
        page.width,
        page.height,
      ),
    );
  }
  function editorKey(e: KeyboardEvent) {
    if (
      mode !== 'Editor' ||
      saving ||
      discarding ||
      blocked ||
      document.querySelector('[role=dialog],.combo-popup')
    )
      return;
    if (e.key === 'Escape' && gesture) {
      e.preventDefault();
      e.stopPropagation();
      cancelGesture();
      return;
    }
    if ((e.target as HTMLElement)?.closest('input,textarea,[contenteditable=true],[role=combobox]'))
      return;
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'z') {
      e.preventDefault();
      void act(() => history(!e.shiftKey));
    }
  }
  async function cleaned() {
    if (!page || !project) return;
    const target = { pageId: page.id, path: project.path, generation: editor.generation };
    const picked = await open({
      title: tr('Import cleaned background', $locale),
      filters: [{ name: tr('Cleaned background', $locale), extensions: ['png', 'jpg', 'webp'] }],
    });
    if (
      typeof picked === 'string' &&
      page?.id === target.pageId &&
      project?.path === target.path &&
      editor.generation === target.generation
    )
      backgroundPrompt = { ...target, background: picked };
  }
  async function confirmBackground() {
    const prompt = backgroundPrompt;
    if (
      !prompt ||
      !page ||
      project?.path !== prompt.path ||
      page.id !== prompt.pageId ||
      editor.generation !== prompt.generation
    )
      return;
    const draft = structuredClone($state.snapshot(page));
    draft.background = prompt.background;
    await saveDraft(draft, { background: true });
    backgroundPrompt = null;
  }
  async function deleteRegion() {
    const id = deletePrompt;
    if (!id || !page) return;
    const draft = structuredClone($state.snapshot(page));
    draft.regions = draft.regions.filter((r) => r.id !== id);
    if (
      editor.baseline?.regions.some((r) => r.id === id) ||
      savedPage?.regions.some((r) => r.id === id)
    )
      await saveDraft(draft, { regionId: id });
    else {
      editor.draft = draft;
      editor.dirty = !!editor.baseline && hasDraftChanges($state.snapshot(editor.baseline), draft);
      geometryUndo = geometryUndo.filter((e) => e.regionId !== id);
      geometryRedo = geometryRedo.filter((e) => e.regionId !== id);
      editor.selectedRegion = Math.max(
        0,
        Math.min(editor.selectedRegion, draft.regions.length - 1),
      );
    }
    deletePrompt = null;
  }
  async function translatePage() {
    if (
      preparing ||
      !translationReadiness.ready('translate') ||
      !(await beforeLeave()) ||
      !page ||
      !project
    )
      return;
    const target = {
      id: page.id,
      revision: page.revision,
      path: project.path,
      action: 'translate' as const,
    };
    if (page.regions.length || page.rendered || page.cleanup || page.background)
      preparationPrompt = target;
    else await submitPrepare(target, false);
  }
  async function requestRegion(action: 'read' | 'translate') {
    if (
      !page ||
      !region ||
      !project ||
      regionOps.busy ||
      saving ||
      discarding ||
      pageBusy ||
      jobsHeld
    )
      return;
    if (!regionReadiness.ready(action === 'read' ? 'region_read' : 'region_translate')) return;
    const generation = editor.generation;
    const request = {
      id: randomUuid(),
      path: project.path,
      pageId: page.id,
      expected: page.revision,
      region: structuredClone($state.snapshot(region)),
    };
    const result = await regionOps.run(request, action);
    if (
      !result ||
      disposed ||
      !page ||
      project?.path !== request.path ||
      editor.generation !== generation
    )
      return;
    if (!acceptsRegionResult(request, result, $state.snapshot(page), action)) {
      notify('warning', 'Region changed; result discarded. Retry using the current text and box.');
      return;
    }
    editStart();
    const updated = page!.regions.find((r) => r.id === result.regionId)!;
    if (action === 'read') updated.source = result.text;
    else updated.target = result.text;
    changed();
  }
  async function switchMode(next: string) {
    if (!(await beforeLeave())) return;
    await onmodechange(next);
    mode = next;
    editorMore = false;
    mobileSheet = false;
    if (next === 'Editor') {
      translated = true;
      await editorImage();
    }
  }
  async function toggleControls() {
    const element = readerCanvas?.querySelector<HTMLElement>(`[data-page="${page?.id}"]`);
    const relativeTop =
      element && readerCanvas
        ? element.getBoundingClientRect().top - readerCanvas.getBoundingClientRect().top
        : 0;
    collapsed = !collapsed;
    showPages = false;
    await tick();
    if (element && readerCanvas)
      readerCanvas.scrollTop +=
        element.getBoundingClientRect().top -
        readerCanvas.getBoundingClientRect().top -
        relativeTop;
  }
  function readerKey(e: KeyboardEvent) {
    if (
      mode !== 'Reader' ||
      blocked ||
      e.defaultPrevented ||
      e.isComposing ||
      e.ctrlKey ||
      e.metaKey ||
      e.altKey
    )
      return;
    const target = e.target as HTMLElement;
    if (
      target?.closest(
        'input,textarea,select,[contenteditable=true],[role=combobox],[role=listbox]',
      ) ||
      document.querySelector('[role=dialog],.combo-popup')
    )
      return;
    if (e.key === 'Escape') {
      if (showPages) showPages = false;
      else if (fullscreen) onexitfullscreen();
      else if (collapsed) void toggleControls();
      else return;
      e.preventDefault();
    } else if (e.key === 'F11' && (!hosted || document.fullscreenEnabled)) {
      e.preventDefault();
      onfullscreen();
    } else if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') {
      e.preventDefault();
      const delta = e.key === 'ArrowRight' ? 1 : -1;
      if (page && project && index + delta >= 0 && index + delta < project.pages.length)
        act(() => navigate(index + delta));
      else if (delta > 0)
        readerCanvas?.querySelector('.chapter-end')?.scrollIntoView({ block: 'end' });
    }
  }
  let readingPointer: { id: number; x: number; y: number; scroll: number; left: number } | null =
    null;
  function readingDown(e: PointerEvent) {
    if (
      !collapsed ||
      e.button !== 0 ||
      (e.target as HTMLElement).closest('button,input,a,[role=combobox]')
    ) {
      readingPointer = null;
      return;
    }
    readingPointer = {
      id: e.pointerId,
      x: e.clientX,
      y: e.clientY,
      scroll: readerCanvas?.scrollTop || 0,
      left: readerCanvas?.scrollLeft || 0,
    };
  }
  function readingUp(e: PointerEvent) {
    const down = readingPointer;
    readingPointer = null;
    if (
      !down ||
      !collapsed ||
      !readerCanvas ||
      down.id !== e.pointerId ||
      Math.hypot(e.clientX - down.x, e.clientY - down.y) > 6 ||
      Math.abs(readerCanvas.scrollTop - down.scroll) > 3 ||
      Math.abs(readerCanvas.scrollLeft - down.left) > 3 ||
      window.getSelection()?.toString()
    )
      return;
    const rect = readerCanvas.getBoundingClientRect(),
      at = (e.clientX - rect.left) / rect.width;
    if (continuous || (at >= 0.2 && at <= 0.8)) {
      void toggleControls();
      return;
    }
    const delta = at < 0.2 ? -1 : 1;
    if (project && index + delta >= 0 && index + delta < project.pages.length)
      act(() => navigate(index + delta));
    else if (delta > 0)
      readerCanvas.querySelector('.chapter-end')?.scrollIntoView({ block: 'end' });
  }
  $effect(() => {
    continuous;
    mode;
    readerCanvas;
    untrack(() => {
      if (mode !== 'Reader' || !continuous || !readerCanvas) return;
      const desiredPage = page?.id;
      positioningReader = true;
      void tick().then(() => {
        if (mode === 'Reader' && continuous)
          readerCanvas
            ?.querySelector(`[data-page="${desiredPage}"]`)
            ?.scrollIntoView({ block: 'start' });
        positioningReader = false;
      });
    });
  });
  const pendingRefresh = new Set<string>();
  const invalidatedPages = new Set<string>();
  let refreshTimer: ReturnType<typeof setTimeout> | undefined;
  function applyPageUpdate(updated: Page) {
    if (!project || disposed) return;
    const position = project.pages.findIndex((p) => p.id === updated.id);
    if (position < 0 || updated.revision <= project.pages[position].revision) return;
    const selectedId = position === index ? region?.id : null;
    project.pages[position] = updated;
    if (position === index && !saving) {
      if (hosted && editor.dirty && editor.baseline && editor.draft) {
        remoteSaved = updated;
        // The old baseline must remain unacknowledged until this browser reviews the save.
        void act(editorImage);
        return;
      }
      if (editor.dirty && editor.baseline && editor.draft) {
        // Preserve only intentional edits; accept newly detected/translated regions.
        editor.draft = mergePageEdits(
          $state.snapshot(editor.baseline),
          $state.snapshot(editor.draft),
          updated,
        );
        editor.baseline = structuredClone(updated);
      } else {
        editor.draft = null;
        editor.baseline = null;
      }
      if (selectedId)
        editor.selectedRegion = Math.max(
          0,
          page!.regions.findIndex((r) => r.id === selectedId),
        );
      void act(editorImage);
    }
  }
  const refreshPageRequest = refreshQueue(async (key: string) => {
    const [path, id] = JSON.parse(key) as [string, string];
    if (disposed || project?.path !== path) return;
    try {
      const updated = await call('page_get', { path, pageId: id });
      if (disposed || project?.path !== path) return;
      invalidatedPages.delete(id);
      if (gesture?.pageId === id) {
        const pending = deferredPages.get(id);
        if (!pending || updated.revision > pending.revision) deferredPages.set(id, updated);
        return;
      }
      applyPageUpdate(updated);
    } catch (e) {
      if (!disposed && project?.path === path) fail(e);
    }
  });
  async function refreshPage(id: string) {
    if (project) await refreshPageRequest(JSON.stringify([project.path, id]));
  }
  $effect(() => {
    const revisions = new Map(project?.pages.map((p) => [p.id, p.revision]));
    const updates = jobs
      .filter((j) => j.project === project?.path && j.pageRevision != null)
      .filter((j) => (revisions.get(j.pageId) ?? Infinity) < j.pageRevision!);
    // Do not restart this timer for every tile update: a long run must not starve refreshes.
    for (const j of updates) pendingRefresh.add(j.pageId);
    if (pendingRefresh.size && !refreshTimer)
      refreshTimer = setTimeout(() => {
        refreshTimer = undefined;
        const ids = [...pendingRefresh];
        pendingRefresh.clear();
        for (const id of ids) void refreshPage(id);
      }, 80);
  });
  onMount(() => {
    const stopViewport = observeViewport(positionWorkspaceOverlays);
    const outsideEditorMore = (event: PointerEvent) => {
      if (
        editorMore &&
        !morePanel?.contains(event.target as Node) &&
        !moreTrigger?.contains(event.target as Node)
      )
        editorMore = false;
    };
    document.addEventListener('pointerdown', outsideEditorMore, true);
    let stopContent: (() => void) | undefined;
    const resync = () => {
      invalidatedPages.clear();
      for (const item of project?.pages || []) invalidatedPages.add(item.id);
      if (page) void refreshPage(page.id);
    };
    window.addEventListener('umanga-host-resync', resync);
    void listenContent((change) => {
      if (change.path === project?.path && change.pageId) void refreshPage(change.pageId);
    }).then((stop) => {
      if (disposed) stop();
      else stopContent = stop;
    });
    act(editorImage);
    const warnUnsaved = (e: BeforeUnloadEvent) => {
      if (editor.dirty) {
        e.preventDefault();
        e.returnValue = '';
      }
    };
    window.addEventListener('beforeunload', warnUnsaved);
    return () => {
      stopViewport();
      document.removeEventListener('pointerdown', outsideEditorMore, true);
      disposed = true;
      stopContent?.();
      window.removeEventListener('umanga-host-resync', resync);
      invalidateEditorImage();
      void regionOps.cancel();
      clearTimeout(refreshTimer);
      resolveLeave?.(false);
      window.removeEventListener('beforeunload', warnUnsaved);
    };
  });
</script>

{#snippet geometryTools()}
  <div class="editor-tool-group">
    <ComboBox
      label={tr('Editor tool', $locale)}
      value={tool}
      disabled={regionOps.busy || saving || discarding}
      oncommit={(value) => {
        cancelGesture();
        tool = value;
        regionAction = 'move';
      }}
      options={[
        { value: 'select', label: 'Select region', disabled: false },
        { value: 'edit', label: 'Edit region', disabled: false },
      ]}
    />{#if tool === 'edit'}<div
        class="region-action-buttons"
        role="group"
        aria-label={tr('Edit region', $locale)}
      >
        <button
          class:active={regionAction === 'draw'}
          aria-pressed={regionAction === 'draw'}
          disabled={regionOps.busy || saving || discarding}
          title={tr('Drag to draw a text region', $locale)}
          onclick={drawRegion}>{tr('Draw', $locale)}</button
        >
        <button
          class:active={regionAction === 'move'}
          aria-pressed={regionAction === 'move'}
          disabled={regionOps.busy || saving || discarding}
          title={tr('Drag a region to move it; drag its handles to resize', $locale)}
          onclick={() => {
            cancelGesture();
            regionAction = 'move';
          }}>{tr('Move', $locale)}</button
        >
      </div>{/if}
  </div>
{/snippet}

<svelte:window
  onkeydown={(e) => {
    editorKey(e);
    readerKey(e);
  }}
/>
<div class="chapter-workspace" class:compact-workspace={compact.current}>
  {#if mode === 'Editor' && project}
    <div
      class="reader-tools editor-navigation"
      inert={sheetActive}
      role="toolbar"
      aria-label={t('m_66eca9a928e8', $locale)}
    >
      <button
        class="editor-back"
        aria-label={t('m_42d2d0b686bc', $locale)}
        title={t('m_42d2d0b686bc', $locale)}
        onclick={() =>
          act(async () => {
            if (await beforeLeave()) onback();
          })}><ArrowLeft size={18} /></button
      >
      <span class="editor-chapter-name" title={project.title}>{project.title}</span>
      <PageViewSelector {translated} onchange={(value) => act(() => selectView(value))} />
      <div class="editor-page-navigation">
        <button
          title={t('m_1208ec01f223', $locale)}
          aria-label={t('m_1208ec01f223', $locale)}
          disabled={index === 0}
          onclick={() => act(() => navigate(index - 1))}><ChevronLeft size={18} /></button
        >
        <span>{index + 1} / {project.pages.length}</span>
        <button
          title={t('m_c08ac736a5e2', $locale)}
          aria-label={t('m_c08ac736a5e2', $locale)}
          disabled={index === project.pages.length - 1}
          onclick={() => act(() => navigate(index + 1))}><ChevronRight size={18} /></button
        >
      </div>
      <label class="zoom"
        ><input
          aria-label={t('m_509c517ede79', $locale)}
          title={t('m_1eb8dd1ff1ef', $locale)}
          type="range"
          min="25"
          max="160"
          step="5"
          bind:value={zoom}
        />{zoom}%</label
      >
      {#if !compact.current}<button
          title={t('m_f8c062952076', $locale)}
          onclick={() => act(() => switchMode('Reader'))}
        >
          <BookOpen size={18} />{t('m_76e4a96f8b6a', $locale)}
        </button>{/if}
      {#if page}<div class="editor-page-actions">
          {#if !hosted}<div class="editor-history">
              <button
                aria-label={t('m_a8283ade3185', $locale)}
                title={t('m_35b5ec137ebd', $locale)}
                disabled={regionOps.busy ||
                  saving ||
                  discarding ||
                  (!geometryUndo.length && !editor.undo.length)}
                onclick={() => act(() => history(true))}><Undo2 size={16} /></button
              ><button
                aria-label={t('m_74273989b096', $locale)}
                title={t('m_c886663c1c18', $locale)}
                disabled={regionOps.busy ||
                  saving ||
                  discarding ||
                  (!geometryRedo.length && !editor.redo.length)}
                onclick={() => act(() => history(false))}><Redo2 size={16} /></button
              >
            </div>{/if}
          <div class="editor-process-group" role="group" aria-label={t('m_8fafeca0183d', $locale)}>
            <button
              class="primary auto-translate"
              disabled={regionOps.busy ||
                saving ||
                submittingCount > 0 ||
                preparing ||
                pageBusy ||
                !translationReadiness.ready('translate')}
              title={tr(
                translationReadiness.reason('translate') ||
                  (pageBusy
                    ? 'This page already has a job; use its progress controls'
                    : 'Detect dialogue, read, translate, clean, and letter this page'),
                $locale,
              )}
              onclick={() => act(translatePage)}
              aria-label={t('m_1f48b54fabdf', $locale)}
              aria-busy={preparing || submittingCount > 0}
              ><ScanText size={19} />{compact.current
                ? tr('Auto translate', $locale)
                : t('m_1f48b54fabdf', $locale)}</button
            >
            {#if !compact.current}<button
                class="prepare-page"
                disabled={regionOps.busy ||
                  saving ||
                  preparing ||
                  pageBusy ||
                  !readiness.ready('prepare')}
                title={tr(
                  readiness.reason('prepare') ||
                    'Detect, recognize, and clean this page without a translation service',
                  $locale,
                )}
                onclick={() => act(preparePage)}
                aria-busy={preparing}>{t('m_50440c66a7da', $locale)}</button
              >{/if}
          </div>
          {#if !hosted}<button
              class="import-background"
              title={tr('Choose a cleaned page background and review before applying', $locale)}
              disabled={regionOps.busy || saving || discarding}
              onclick={() => act(cleaned)}
              ><ImagePlus size={16} />{t('m_3e41a5201025', $locale)}</button
            >
          {/if}
          <div class="editor-progress-tools">
            <EditorProgress
              pageId={page.id}
              job={pageJob}
              operation={regionOps.state}
              oncancel={() => void regionOps.cancel()}
              held={jobsHeld}
              available={!!pageJob && jobAvailability.ready(pageJob.id)}
              reason={pageJob ? jobAvailability.reason(pageJob.id) : ''}
              hint={pageJob ? jobAvailability.hint(pageJob.id) : ''}
              oncontrol={onjobcontrol}
              {onsettings}
            />
            {#if !hosted}<button
                class="editor-glossary"
                title={tr('Inspect and edit terms for this book', $locale)}
                onclick={onglossary}><BookA size={16} />{tr('Glossary', $locale)}</button
              >{/if}
          </div>
        </div>
        <div class="editor-page-hints" class:compact-hints={compact.current}>
          <RequirementHint reason={translationReadiness.hint('translate')} {onsettings} />
          {#if readiness.hint('prepare') !== translationReadiness.hint('translate')}<RequirementHint
              reason={readiness.hint('prepare')}
              {onsettings}
            />{/if}
        </div>{/if}
      {#if compact.current}<button
          class="editor-more-trigger"
          bind:this={moreTrigger}
          aria-label={tr('Page options', $locale)}
          title={tr('Page options', $locale)}
          aria-haspopup="dialog"
          aria-expanded={editorMore}
          onclick={() => void toggleEditorMore()}><MoreHorizontal size={20} /></button
        >{/if}
    </div>
    {#if compact.current && editorMore}<div
        class="mobile-editor-options"
        bind:this={morePanel}
        role="dialog"
        aria-modal="false"
        aria-label={tr('Page options', $locale)}
        tabindex="-1"
        onkeydown={editorMoreKey}
        style:left={`${moreBounds.left}px`}
        style:top={`${moreBounds.top}px`}
        style:width={`${moreBounds.width}px`}
        style:max-height={`${moreBounds.maxHeight}px`}
      >
        <button
          class="host-edit-text-button"
          onclick={() =>
            editorOption(() => {
              mobileSheet = true;
            })}><PenLine size={18} />{t('host.editText', $locale)}</button
        >
        <button
          disabled={regionOps.busy ||
            saving ||
            preparing ||
            pageBusy ||
            !readiness.ready('prepare')}
          title={tr(
            readiness.reason('prepare') ||
              'Detect, recognize, and clean this page without a translation service',
            $locale,
          )}
          aria-busy={preparing}
          onclick={() =>
            editorOption(() => {
              void act(preparePage);
            })}>{t('m_50440c66a7da', $locale)}</button
        >
        {#if translationReadiness.hint('translate') || readiness.hint('prepare')}<p
            class="mobile-action-hint"
          >
            {tr(translationReadiness.hint('translate') || readiness.hint('prepare'), $locale)}
            {tr('Configure missing models or services on the PC.', $locale)}
          </p>{/if}
        <div class="mobile-options-divider"></div>
        <button
          onclick={() =>
            editorOption(() => {
              void act(() => switchMode('Reader'));
            })}><BookOpen size={18} />{t('m_76e4a96f8b6a', $locale)}</button
        >
        <button
          aria-pressed={editorThumbnails}
          onclick={() =>
            editorOption(() => {
              editorThumbnails = !editorThumbnails;
            })}
          ><PanelBottom size={18} />{t('m_d82fd62f9623', $locale)}{#if editorThumbnails}<Check
              size={16}
            />{/if}</button
        >
        <label class="mobile-editor-zoom"
          >{t('m_8bc067ef2d85', $locale)}
          {zoom}%<input
            aria-label={t('m_509c517ede79', $locale)}
            type="range"
            min="25"
            max="160"
            step="5"
            bind:value={zoom}
          /></label
        >
      </div>{/if}
  {/if}
  {#if mode === 'Reader' && !collapsed && project}<ReaderControls
      {index}
      total={project.pages.length}
      bind:continuous
      bind:zoom
      {translated}
      translating={readerTranslate}
      translationReason={translationReadiness.displayReason('translate')}
      {showPages}
      night={settings.appearance.active === 'night'}
      {fullscreen}
      {onback}
      onnavigate={(i) => act(() => navigate(i))}
      onview={(value) => act(() => selectView(value))}
      ontranslation={(enabled) => {
        readerTranslate = enabled;
        if (enabled && translationReadiness.ready('translate')) act(prefetch);
      }}
      oncollapse={() => void toggleControls()}
      onpages={(visible) => (showPages = visible)}
      {ontheme}
      {onfullscreen}
      oneditor={() => act(() => switchMode('Editor'))}
      {onexport}
      {onsettings}
    />{/if}
  {#if project && thumbnailsMounted}<div
      class="workspace-thumbnails"
      class:thumbnails-hidden={!thumbnailsShown}
      aria-hidden={!thumbnailsShown}
      inert={sheetActive || !thumbnailsShown}
    >
      <PageStrip
        pages={project.pages}
        path={project.path}
        {index}
        onnavigate={(i) => act(() => navigate(i))}
      />
    </div>{/if}
  {#if project && page}
    {#if mode === 'Reader'}<div
        class="reader-canvas"
        role="region"
        aria-label={t('m_efdaee70e0e7', $locale)}
        onpointerdown={readingDown}
        onpointerup={readingUp}
        onpointercancel={() => (readingPointer = null)}
        use:fitSurface
        bind:this={readerCanvas}
        style:background={theme.readerBackground}
        style:padding={`${theme.margin}px`}
        style:gap={`${theme.gap}px`}
      >
        {#if continuous}{#each project.pages as p}<PageView
              page={p}
              path={project.path}
              {translated}
              filter={pageFilter(theme, mode)}
              {width}
              onerror={fail}
              onvisible={(id) => {
                if (positioningReader) return;
                const i = project!.pages.findIndex((p) => p.id === id);
                if (i !== index) {
                  selectPage(i);
                  if (readerTranslate) act(prefetch);
                }
              }}
            />{/each}{:else}{#key page.id}<PageView
              {page}
              path={project.path}
              {translated}
              filter={pageFilter(theme, mode)}
              {width}
              onerror={fail}
            />{/key}{/if}
        {#if continuous || index === project.pages.length - 1}<div
            class="chapter-end"
            use:endVisible
          >
            <button onclick={onback}>{t('m_42d2d0b686bc', $locale)}</button>{#if hasNext}<button
                onclick={onnext}>{t('m_4344dde797b6', $locale)}</button
              >{/if}
          </div>{/if}
      </div>
      {#if readerTranslate}<ReaderJobBadge
          job={pageJob}
          pending={readerPending}
          held={jobsHeld}
          available={!!pageJob && jobAvailability.ready(pageJob.id)}
          reason={pageJob ? jobAvailability.reason(pageJob.id) : ''}
          hint={pageJob ? jobAvailability.hint(pageJob.id) : ''}
          oncontrol={onjobcontrol}
          {onsettings}
        />{/if}
    {:else}<div class="editor-layout">
        <div class="editor-main" inert={sheetActive}>
          {#if !hosted}<section class="geometry-toolbox" aria-label={tr('Geometry', $locale)}>
              <button
                class="geometry-toolbox-heading"
                aria-expanded={geometryExpanded}
                onclick={() => (geometryExpanded = !geometryExpanded)}
              >
                {tr('Geometry', $locale)}
                <ChevronRight
                  size={16}
                  style={geometryExpanded ? 'transform: rotate(90deg)' : ''}
                />
              </button>
              {#if geometryExpanded}
                <div class="geometry-toolbox-body">
                  <fieldset
                    disabled={regionOps.busy || saving || discarding || !imageOwned}
                    onfocusin={editStart}
                  >
                    {@render geometryTools()}
                    <div class="form-grid">
                      {#each ['Left', 'Top', 'Right', 'Bottom'] as name, i}<label
                          >{tr(name, $locale)}<input
                            type="number"
                            min="0"
                            value={region?.bbox[i] ?? ''}
                            disabled={!region}
                            onchange={(e) => coordinate(i, e.currentTarget.valueAsNumber)}
                          /></label
                        >{/each}
                    </div>
                  </fieldset>
                </div>
              {/if}
            </section>{/if}
          <div class="editor-canvas" use:fitSurface>
            <div
              class="paper editor-paper"
              class:drawing={tool === 'edit' && regionAction === 'draw'}
              class:moving={tool === 'edit' && regionAction === 'move'}
              bind:this={canvas}
              style:width={`${width}px`}
              style:aspect-ratio={`${page.width}/${page.height}`}
              style:visibility={imageOwned || (hosted && !!editorSrc) ? 'visible' : 'hidden'}
              onpointerdown={down}
              onpointermove={move}
              onpointerup={up}
              onpointercancel={cancelGesture}
              onlostpointercapture={() => {
                if (gesture) cancelGesture();
              }}
              role="presentation"
            >
              {#if hosted}<img
                  bind:this={webEditorImage}
                  src={editorSrc}
                  alt={t('m_ab2ddb05d85a', $locale)}
                  draggable="false"
                  onload={webImageReady}
                  onerror={() => {
                    if (
                      !disposed &&
                      editorSrc &&
                      webEditorImage?.src === new URL(editorSrc, document.baseURI).href
                    )
                      fail(Error('Page image could not be loaded.'));
                  }}
                />
              {:else}{#key imageRequest}
                  {@const request = imageRequest}
                  <img
                    src={editorSrc}
                    alt={t('m_ab2ddb05d85a', $locale)}
                    draggable="false"
                    onload={() => {
                      if (!disposed && request === imageRequest && editorSrc) imageReady = true;
                    }}
                    onerror={() => {
                      if (!disposed && request === imageRequest && editorSrc)
                        fail(Error('Page image could not be loaded.'));
                    }}
                  />
                {/key}{/if}{#each page.regions as r, i (r.id)}{@const box = boxFor(r)}<button
                  class="region-box"
                  data-region-id={r.id}
                  onkeydown={(e) => regionKey(e, r.id)}
                  class:selected={editor.selectedRegion === i}
                  aria-label={tr(`Edit region ${i + 1}`, $locale)}
                  title={tr(`Edit region ${i + 1}`, $locale)}
                  style:left={`${(box[0] / page.width) * 100}%`}
                  style:top={`${(box[1] / page.height) * 100}%`}
                  style:width={`${((box[2] - box[0]) / page.width) * 100}%`}
                  style:height={`${((box[3] - box[1]) / page.height) * 100}%`}
                  onclick={() => {
                    if (tool === 'select' || regionAction === 'move') editor.selectedRegion = i;
                    if (hosted) mobileSheet = true;
                  }}><span>{i + 1}</span></button
                >
                {#if tool === 'edit' && regionAction === 'move' && editor.selectedRegion === i}
                  <div
                    class="region-handles"
                    style:left={`${(box[0] / page.width) * 100}%`}
                    style:top={`${(box[1] / page.height) * 100}%`}
                    style:width={`${((box[2] - box[0]) / page.width) * 100}%`}
                    style:height={`${((box[3] - box[1]) / page.height) * 100}%`}
                  >
                    {#each handles as handle}<button
                        class="resize-handle {handle}"
                        data-region-id={r.id}
                        data-resize={handle}
                        disabled={regionOps.busy || saving || discarding}
                        title={tr('Drag to resize region', $locale)}
                        aria-label={tr('Resize region', $locale) + ' ' + handle}
                        onkeydown={(e) => regionKey(e, r.id)}
                      ></button>{/each}
                  </div>
                {/if}
              {/each}
              {#if gesture?.action === 'draw'}<div
                  class="region-draft"
                  style:left={`${(gesture.box[0] / page.width) * 100}%`}
                  style:top={`${(gesture.box[1] / page.height) * 100}%`}
                  style:width={`${((gesture.box[2] - gesture.box[0]) / page.width) * 100}%`}
                  style:height={`${((gesture.box[3] - gesture.box[1]) / page.height) * 100}%`}
                ></div>{/if}
            </div>
          </div>
        </div>
        {#if sheetActive}<button
            class="mobile-sheet-scrim"
            tabindex="-1"
            aria-label={t('host.close', $locale)}
            style:z-index={60 + sheetRank}
            inert={!sheetTop}
            onclick={() => (mobileSheet = false)}
          ></button>{/if}
        <!-- svelte-ignore a11y_no_noninteractive_tabindex a11y_no_noninteractive_element_interactions (The compact inspector is a modal text editor, with the desktop complementary role preserved.) -->
        <aside
          class="inspector"
          class:sheet-open={sheetActive}
          bind:this={inspector}
          role={sheetActive ? 'dialog' : 'complementary'}
          aria-modal={sheetActive ? true : undefined}
          aria-label={sheetActive ? t('host.editText', $locale) : undefined}
          tabindex={sheetActive ? -1 : undefined}
          inert={compact.current && (!sheetActive || !sheetTop)}
          aria-hidden={compact.current && (!sheetActive || !sheetTop)}
          data-modal-owner={sheetActive ? sheetOwner : undefined}
          onkeydown={sheetKey}
          style:left={sheetActive ? `${sheetBounds.left}px` : undefined}
          style:top={sheetActive ? `${sheetBounds.top}px` : undefined}
          style:width={sheetActive ? `${sheetBounds.width}px` : undefined}
          style:height={sheetActive ? `${sheetBounds.height}px` : undefined}
          style:z-index={sheetActive ? 61 + sheetRank : undefined}
        >
          {#if compact.current}<div class="host-sheet-heading">
              <strong>{t('host.editText', $locale)}</strong><button
                onclick={() => (mobileSheet = false)}
                aria-label={t('host.close', $locale)}>✕</button
              >
            </div>{/if}
          {#if hosted && remoteSaved}
            {@const current = remoteSaved.regions.find((r) => r.id === region?.id)}
            <div class="host-editor-conflict" role="alert">
              <p>{t('host.editConflict', $locale)}</p>
              {#if current}<details>
                  <summary>{t('host.savedText', $locale)}</summary>
                  <pre>{current.source}</pre>
                  <pre>{current.target}</pre>
                </details>{/if}
              <button onclick={reviewRemote}>{t('host.reviewKeep', $locale)}</button>
            </div>
          {/if}
          <div class="inspector-scroll">
            {#if dialogError && dialogErrorOwner === 'sheet'}<p class="dialog-error" role="alert">
                {tr(dialogError, $locale)}
              </p>{/if}
            <fieldset
              class="editor-fields"
              disabled={saving || discarding || !imageOwned}
              onfocusin={editStart}
            >
              <div class="sidebar-head">
                <span>{t('m_90b4c2453024', $locale)}</span>{#if !hosted}<button
                    aria-label={t('m_fb6cc173a144', $locale)}
                    title={t('m_f56a1ffaab52', $locale)}
                    onclick={drawRegion}><Plus size={16} /></button
                  >{/if}
              </div>
              <ComboBox
                label={tr('Selected region', $locale)}
                bind:value={editor.selectedRegion}
                options={[
                  ...page.regions.flatMap((r, i) => [
                    {
                      value: i,
                      literal: true,
                      label: (
                        (dirtyRegionIds.has(r.id) ? '• ' : '') +
                        String(i + 1) +
                        ' · ' +
                        String(
                          r.target.slice(0, 14) ||
                            r.source.slice(0, 14) ||
                            tr('Untranslated', $locale),
                        )
                      ).trim(),
                      disabled: false,
                    },
                  ]),
                ]}
              />{#if region}<div class="region-selection-actions">
                  <button
                    class="region-delete danger"
                    disabled={regionOps.busy || saving || discarding || !!editor.pendingHistory}
                    title={tr('Delete only the selected region', $locale)}
                    onclick={() => (deletePrompt = region!.id)}
                    ><Trash2 size={14} />{t('m_15d571978505', $locale)}</button
                  >
                </div>{/if}{#if region}<div oninput={changed} onchange={changed}>
                  <section class="region-group region-text" aria-label={tr('Text', $locale)}>
                    <div class="region-field-heading">
                      <label for="region-source">{t('m_0e570ca6fabe', $locale)}</label>
                      {#if !hosted}<button
                          class="region-text-action"
                          aria-label={tr('Re-read text (OCR)', $locale)}
                          disabled={regionOps.busy ||
                            saving ||
                            pageBusy ||
                            jobsHeld ||
                            !regionReadiness.ready('region_read')}
                          title={tr(
                            jobsHeld
                              ? 'Use Start all in Jobs first.'
                              : regionReadiness.reason('region_read') ||
                                  'Read only this region into the source draft; translation is unchanged',
                            $locale,
                          )}
                          onclick={() => act(() => requestRegion('read'))}
                          ><ScanText size={14} />{tr('OCR', $locale)}</button
                        >{/if}
                    </div>
                    <textarea id="region-source" bind:value={region.source} rows="3"></textarea>
                    <div class="region-field-heading">
                      <label for="region-target">{t('m_6fbd766b1a71', $locale)}</label>
                      {#if !hosted}<button
                          class="region-text-action"
                          aria-label={tr('Re-translate', $locale)}
                          disabled={regionOps.busy ||
                            saving ||
                            pageBusy ||
                            jobsHeld ||
                            !region.source.trim() ||
                            !regionReadiness.ready('region_translate')}
                          title={tr(
                            jobsHeld
                              ? 'Use Start all in Jobs first.'
                              : !region.source.trim()
                                ? 'Enter source text before translating this region'
                                : regionReadiness.reason('region_translate') ||
                                  'Translate the source text above; no OCR or image upload',
                            $locale,
                          )}
                          onclick={() => act(() => requestRegion('translate'))}
                          ><Languages size={14} />{tr('Translate', $locale)}</button
                        >{/if}
                    </div>
                    <textarea id="region-target" bind:value={region.target} rows="4"></textarea>
                    {#if !hosted}<RequirementHint
                        reason={regionReadiness.hint('region_read')}
                        {onsettings}
                      />
                      <RequirementHint
                        reason={regionReadiness.hint('region_translate')}
                        {onsettings}
                      />{/if}
                    {#if region.review}<p class="review-note">{tr(region.review, $locale)}</p>
                    {/if}
                  </section>
                  {#if !hosted}
                    <section
                      class="region-group region-lettering"
                      aria-label={tr('Lettering', $locale)}
                    >
                      <h4>{tr('Lettering', $locale)}</h4>
                      <label
                        >{t('m_0935fd222194', $locale)}<ComboBox
                          label={tr('Writing direction', $locale)}
                          bind:value={region.direction}
                          oncommit={() => changed()}
                          options={[
                            { value: 'auto', label: 'Auto · region proportions', disabled: false },
                            {
                              value: 'vertical',
                              label: 'Vertical · right to left columns',
                              disabled: false,
                            },
                            { value: 'horizontal', label: 'Horizontal', disabled: false },
                            {
                              value: 'rtl',
                              label: 'Horizontal · bidirectional',
                              disabled: false,
                            },
                          ]}
                        /></label
                      ><label
                        >{t('m_64d0b3adcd2d', $locale)}<ComboBox
                          label={t('m_64d0b3adcd2d', $locale)}
                          editable
                          value={region.style.font || ''}
                          placeholder={t('m_cc825ce17b03', $locale)}
                          options={fonts.map((f) => ({ value: f, label: f, literal: true }))}
                          oninput={(value) => {
                            editStart();
                            region!.style.font = value || null;
                            changed();
                          }}
                        /></label
                      >
                      <div class="form-grid">
                        <label
                          >{t('m_91ccc4d96c62', $locale)}<input
                            type="number"
                            min="6"
                            max="120"
                            value={region.style.size || ''}
                            oninput={(e) =>
                              (region!.style.size = e.currentTarget.value
                                ? Number(e.currentTarget.value)
                                : null)}
                          /></label
                        ><label
                          >{t('m_4a69d0166046', $locale)}<input
                            type="color"
                            bind:value={region.style.color}
                          /></label
                        >
                      </div>
                      <section class="outline-controls" aria-label={t('outline.enabled', $locale)}>
                        <ToggleSwitch
                          wide
                          label={t('outline.enabled', $locale)}
                          title={t('outline.enabled', $locale)}
                          checked={region.style.outlineEnabled}
                          onchange={(enabled) => {
                            editStart();
                            region!.style.outlineEnabled = enabled;
                            changed();
                          }}
                        />
                        <fieldset class="outline-options" disabled={!region.style.outlineEnabled}>
                          <label
                            >{t('outline.width', $locale)}<input
                              type="number"
                              min="1"
                              max="20"
                              step="1"
                              bind:value={region.style.outlineWidthPercent}
                            /></label
                          >
                          <label
                            >{t('outline.color', $locale)}<input
                              type="color"
                              bind:value={region.style.outlineColor}
                            /></label
                          >
                        </fieldset>
                      </section>
                    </section>
                    <section
                      class="region-group region-cleanup"
                      aria-label={tr('Cleanup', $locale)}
                    >
                      <h4>{tr('Cleanup', $locale)}</h4>
                      <ToggleSwitch
                        wide
                        label={t('m_201b84dbf2f8', $locale)}
                        title={t('m_8e5d75d2aab3', $locale)}
                        checked={region.allowFill}
                        onchange={(value) => {
                          editStart();
                          region!.allowFill = value;
                          changed();
                        }}
                      />
                      <label
                        >{t('m_e743cd5aece3', $locale)}<input
                          type="color"
                          bind:value={region.style.fill}
                        /></label
                      >
                      <ToggleSwitch
                        wide
                        label={t('m_0cb05b743b25', $locale)}
                        title={t('m_e96b784624b3', $locale)}
                        checked={region.overlayOnly}
                        onchange={(value) => {
                          editStart();
                          region!.overlayOnly = value;
                          changed();
                        }}
                      />
                    </section>
                  {/if}
                </div>
              {:else}
                <div class="empty"><p>{t('m_113e98714a84', $locale)}</p></div>{/if}
            </fieldset>
          </div>
          {#if region || (editor.pendingHistory && editor.dirty)}
            <div class="region-save-bar editor-confirm">
              <div class="region-save-status">
                <small
                  >{saving
                    ? t('m_23e39291d613', $locale)
                    : confirmationDirty
                      ? tr('Unsaved', $locale)
                      : t('m_b5c120b316c2', $locale)}</small
                >
              </div>
              <div class="region-save-actions">
                <button
                  disabled={regionOps.busy || !confirmationDirty || saving || discarding}
                  title={tr(
                    confirmationAll
                      ? 'Discard all'
                      : editor.pendingHistory
                        ? 'Discard the pending history change'
                        : 'Discard changes to this region only',
                    $locale,
                  )}
                  onclick={() => act(() => discardEdits(confirmationScope))}
                  >{confirmationAll
                    ? tr('Discard all', $locale)
                    : t('m_eb1a70e39274', $locale)}</button
                >
                <button
                  class="primary"
                  disabled={regionOps.busy ||
                    !confirmationDirty ||
                    saving ||
                    discarding ||
                    !selectedReadiness.ready('edit') ||
                    (hosted && !!remoteSaved)}
                  title={tr(
                    selectedReadiness.reason('edit') ||
                      (confirmationAll
                        ? 'Save all'
                        : editor.pendingHistory
                          ? 'Confirm the pending history change'
                          : 'Save this region and update the page preview'),
                    $locale,
                  )}
                  onclick={() => act(() => confirmEdits(confirmationScope))}
                  ><Check size={16} />{tr(confirmationAll ? 'Save all' : 'Save', $locale)}</button
                >
              </div>
              {#if confirmationDirty}<RequirementHint
                  reason={selectedReadiness.hint('edit')}
                  {onsettings}
                />{/if}
            </div>
          {/if}
        </aside>
      </div>{/if}{/if}
  {#if busy}<div class="busy">{t('m_a2dd9cbb7013', $locale)}</div>{/if}
</div>

{#if leavePrompt}<Modal
    title={t('m_27a305cb1c09', $locale)}
    onclose={() => void resolveEdits('stay')}
  >
    <p>
      {tr('This page has unsaved region changes. Save or discard them before leaving?', $locale)}
    </p>
    {#if dialogError && dialogErrorOwner === 'leave'}<p class="dialog-error" role="alert">
        {tr(dialogError, $locale)}
      </p>{/if}
    {#snippet footer()}
      <button
        disabled={regionOps.busy || saving || discarding}
        onclick={() => void resolveEdits('stay')}>{t('m_e76fd2add010', $locale)}</button
      >
      <span class="spacer"></span>
      <button
        disabled={regionOps.busy || saving || discarding}
        onclick={() => act(() => resolveEdits('discard'))}>{tr('Discard all', $locale)}</button
      >
      <button
        class="primary"
        disabled={regionOps.busy || saving || !readiness.ready('edit')}
        onclick={() => act(() => resolveEdits('confirm'))}
        >{saving ? t('m_23e39291d613', $locale) : tr('Save all', $locale)}</button
      >
    {/snippet}
  </Modal>{/if}
{#if backgroundPrompt}<Modal
    title={tr('Apply cleaned background?', $locale)}
    closable={!saving}
    onclose={() => {
      if (!saving) {
        backgroundPrompt = null;
        editor.pendingHistory = null;
      }
    }}
  >
    <p>
      {tr('Replace this page’s background. Unsaved region changes stay in the editor.', $locale)}
    </p>
    {#if backgroundPrompt.background}<p class="background-filename">
        {backgroundPrompt.background.split(/[\\/]/).at(-1)}
      </p>{/if}
    <RequirementHint reason={backgroundReadiness.hint('edit')} {onsettings} />
    {#if dialogError && dialogErrorOwner === 'background'}<p class="dialog-error" role="alert">
        {tr(dialogError, $locale)}
      </p>{/if}
    {#snippet footer()}
      <button
        disabled={saving}
        onclick={() => {
          backgroundPrompt = null;
          editor.pendingHistory = null;
        }}>{t('m_19766ed6ccb2', $locale)}</button
      >
      <button
        class="primary"
        disabled={saving || !backgroundReadiness.ready('edit')}
        onclick={() => act(confirmBackground)}
        >{saving ? t('m_23e39291d613', $locale) : tr('Apply', $locale)}</button
      >
    {/snippet}
  </Modal>{/if}
{#if deletePrompt}<Modal
    title={tr('Delete region?', $locale)}
    closable={!saving}
    onclose={() => {
      if (!saving) deletePrompt = null;
    }}
  >
    <p>
      {tr('Delete this region and its text and formatting. Other region drafts are kept.', $locale)}
    </p>
    {#if dialogError && dialogErrorOwner === 'delete'}<p class="dialog-error" role="alert">
        {tr(dialogError, $locale)}
      </p>{/if}
    {#snippet footer()}
      <button disabled={saving} onclick={() => (deletePrompt = null)}
        >{t('m_19766ed6ccb2', $locale)}</button
      >
      <button class="danger" disabled={saving} onclick={() => act(deleteRegion)}
        >{tr('Delete', $locale)}</button
      >
    {/snippet}
  </Modal>{/if}
{#if preparationPrompt}<Modal
    title={tr('Restart this page?', $locale)}
    onclose={() => {
      if (!preparing) preparationPrompt = null;
    }}
  >
    <p>
      {tr(
        'Replace this page’s regions, recognized text, translations, formatting, cleanup, and imported-background association. Original and imported files are kept. Existing work is replaced only after detection succeeds.',
        $locale,
      )}
    </p>
    {#if preparationPrompt.action === 'translate'}<p>
        {tr('This requests fresh translations and may incur service charges.', $locale)}
      </p>{/if}
    {#if dialogError && dialogErrorOwner === 'preparation'}<p class="dialog-error" role="alert">
        {tr(dialogError, $locale)}
      </p>{/if}
    {#snippet footer()}<button disabled={preparing} onclick={() => (preparationPrompt = null)}
        >{t('m_19766ed6ccb2', $locale)}</button
      ><button
        class="primary"
        disabled={preparing}
        onclick={() => act(() => submitPrepare(preparationPrompt!, true))}
        >{tr(
          preparationPrompt?.action === 'translate' ? 'Restart translation' : 'Prepare fresh page',
          $locale,
        )}</button
      >{/snippet}
  </Modal>{/if}
