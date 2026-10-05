import type { Commands } from './commands';
import type { Settings, Page, JobsBatch } from './types';
import type { UiMessage } from './i18n';
import { parseGlossary, encodeGlossary } from './glossary';
import { mergePreferences } from './preferences';
const prefix = '/api/v1';
let csrf = '';
let events: EventSource | undefined;
let epoch = 0,
  serial = 0;
const localKey = 'umanga-browser-v1';
export type BrowserPreferences = Pick<
  Settings,
  'uiLanguage' | 'reader' | 'libraryView' | 'librarySort'
> & { appearance: { active: 'day' | 'night' } };
export const hostedMessage = (fallback: string): UiMessage => ({ key: '', args: {}, fallback });
export async function request<T>(url: string, body?: unknown): Promise<T> {
  let response: Response;
  try {
    response = await fetch(prefix + url, {
      method: body === undefined ? 'GET' : 'POST',
      credentials: 'same-origin',
      cache: 'no-store',
      headers:
        body === undefined ? {} : { 'content-type': 'application/json', 'x-umanga-csrf': csrf },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch {
    window.dispatchEvent(new Event('umanga-connection-lost'));
    throw hostedMessage('Connection lost. Your drafts are retained; reconnect before saving.');
  }
  let data: any;
  try {
    data = await response.json();
  } catch {
    if (response.ok) {
      window.dispatchEvent(new Event('umanga-connection-lost'));
      throw hostedMessage('Connection lost. Your drafts are retained; reconnect before saving.');
    }
  }
  if (!response.ok) {
    if (response.status === 401 && url !== '/login')
      window.dispatchEvent(new Event('umanga-sign-in-required'));
    throw (
      data?.error ||
      hostedMessage(
        response.status === 413
          ? 'This request is too large.'
          : 'Unable to complete this operation.',
      )
    );
  }
  if (data == null) throw hostedMessage('Unable to complete this operation.');
  return data as T;
}
export async function restoreSession() {
  const session = await request<{ csrf: string; epoch: number }>('/session');
  csrf = session.csrf;
  return session;
}
export async function signIn(password: string) {
  const session = await request<{ csrf: string }>('/login', { password });
  csrf = session.csrf;
}
export async function signOut() {
  await request('/logout', {});
  csrf = '';
  events?.close();
  window.dispatchEvent(new Event('umanga-sign-in-required'));
}
export function connectEvents() {
  events?.close();
  const source = (events = new EventSource(prefix + '/events'));
  source.addEventListener('expired', () => {
    if (events !== source) return;
    source.close();
    window.dispatchEvent(new Event('umanga-sign-in-required'));
  });
  source.addEventListener('update', (event) => {
    if (events !== source) return;
    const packet = JSON.parse((event as MessageEvent).data);
    if (packet.epoch && packet.epoch !== epoch) {
      epoch = packet.epoch;
      serial = 0;
    }
    if (packet.serial && packet.serial <= serial) return;
    if (packet.serial) serial = packet.serial;
    if (packet.kind === 'jobs')
      window.dispatchEvent(new CustomEvent('umanga-jobs', { detail: packet.data }));
    if (packet.kind === 'content')
      window.dispatchEvent(
        new CustomEvent('umanga-content-changed', {
          detail: {
            path: packet.data.bookId ? `book:${packet.data.bookId}` : null,
            pageId: packet.data.pageId,
          },
        }),
      );
    if (packet.kind === 'requirements')
      window.dispatchEvent(new Event('umanga-requirements-changed'));
    if (packet.kind === 'resync') {
      void request<JobsBatch>('/jobs')
        .then((batch) => window.dispatchEvent(new CustomEvent('umanga-jobs', { detail: batch })))
        .catch(() => {});
      window.dispatchEvent(new Event('umanga-host-resync'));
      window.dispatchEvent(new Event('umanga-requirements-changed'));
    }
    window.dispatchEvent(new Event('umanga-connection-restored'));
  });
  source.onerror = () => {
    if (events !== source) return;
    window.dispatchEvent(new Event('umanga-connection-lost'));
    void restoreSession().catch(() => {});
  };
  return () => {
    source.close();
    if (events === source) events = undefined;
  };
}
export function applyBrowserPreferences(settings: Settings): Settings {
  const result = structuredClone(settings);
  result.reader.zoom = 100;
  try {
    const local = JSON.parse(localStorage.getItem(localKey) || '{}');
    if (['system', 'en', 'zh-Hans'].includes(local.uiLanguage))
      result.uiLanguage = local.uiLanguage;
    else result.uiLanguage = 'system';
    if (['day', 'night'].includes(local.appearance?.active))
      result.appearance.active = local.appearance.active;
    if (['continuous', 'paged'].includes(local.reader?.layout))
      result.reader.layout = local.reader.layout;
    if (Number.isFinite(local.reader?.zoom) && local.reader.zoom >= 10 && local.reader.zoom <= 300)
      result.reader.zoom = local.reader.zoom;
    if (['grid', 'card', 'list'].includes(local.libraryView))
      result.libraryView = local.libraryView;
    if (['title', 'creator', 'updated'].includes(local.librarySort))
      result.librarySort = local.librarySort;
  } catch {
    /* An invalid browser preference never blocks connection. */
  }
  return result;
}
function saveBrowserPreferences(settings: Settings) {
  const value: BrowserPreferences = {
    uiLanguage: settings.uiLanguage,
    reader: settings.reader,
    libraryView: settings.libraryView,
    librarySort: settings.librarySort,
    appearance: { active: settings.appearance.active },
  };
  localStorage.setItem(localKey, JSON.stringify(value));
  return settings;
}
function bookId(path: unknown) {
  if (typeof path !== 'string' || !/^book:[0-9a-f-]{36}$/i.test(path))
    throw hostedMessage('Book is no longer available; refresh Library.');
  return path.slice(5);
}
function identifier(id: unknown) {
  if (typeof id !== 'string' || !/^[0-9a-f-]{36}$/i.test(id))
    throw hostedMessage('Invalid resource identity.');
  return id;
}
export function regionMutation(page: Page, base: Page, expected: number) {
  if (page.id !== base.id || expected !== base.revision)
    throw hostedMessage('Editor baseline changed.');
  const added = page.regions.filter((r) => !base.regions.some((b) => b.id === r.id));
  const removed = base.regions.filter((r) => !page.regions.some((p) => p.id === r.id));
  const changed = page.regions.filter((r) => {
    const old = base.regions.find((b) => b.id === r.id);
    if (!old) return false;
    const { source: _s, target: _t, ...before } = old,
      { source: _s2, target: _t2, ...after } = r;
    if (JSON.stringify(before) !== JSON.stringify(after))
      throw hostedMessage('Detailed region editing is available on the PC.');
    return r.source !== old.source || r.target !== old.target;
  });
  if (added.length || removed.length + changed.length !== 1)
    throw hostedMessage('Confirm one region at a time.');
  return removed.length
    ? { regionId: removed[0].id, action: 'delete', body: { expected } }
    : {
        regionId: changed[0].id,
        action: 'text',
        body: { expected, source: changed[0].source, target: changed[0].target },
      };
}
export async function hostedCall(command: keyof Commands, a: Record<string, any>): Promise<any> {
  if (command === 'system_language') return navigator.language || 'en';
  if (command === 'bootstrap') {
    const b = await request<Commands['bootstrap']['result']>('/bootstrap');
    b.settings = applyBrowserPreferences(b.settings);
    b.systemLocale = navigator.language || 'en';
    return b;
  }
  if (command === 'preferences') {
    const current = applyBrowserPreferences(a.base || a.settings);
    return saveBrowserPreferences(mergePreferences(a.base || current, a.settings, current));
  }
  if (command === 'library_list') return request('/books');
  if (command === 'jobs_list') return request('/jobs');
  if (command === 'jobs_readiness') return request('/jobs/availability', { ids: a.ids });
  if (command === 'job_control')
    return request(`/jobs/${identifier(a.id)}/control`, { action: a.action });
  if (command === 'jobs_control_all') return request('/jobs/control', { action: a.action });
  if (command === 'glossary_parse') return parseGlossary(a.text, a.format);
  const root = a.path ? `/books/${bookId(a.path)}` : '';
  switch (command) {
    case 'book_open':
      return request(root + (a.glossary ? '/glossary' : ''));
    case 'book_glossary_save':
      return request(root + '/glossary', { expected: a.expected, glossary: a.glossary });
    case 'glossary_export': {
      const url = URL.createObjectURL(
        new Blob([encodeGlossary(a.entries, a.format)], { type: 'text/plain;charset=utf-8' }),
      );
      const link = document.createElement('a');
      link.href = url;
      link.download = a.destination;
      document.body.append(link);
      link.click();
      link.remove();
      setTimeout(() => URL.revokeObjectURL(url), 1000);
      return;
    }
    case 'book_refresh':
      return request(root + '/summary');
    case 'chapter_pages':
      return request(root + `/chapters/${identifier(a.chapterId)}`);
    case 'chapter_complete':
      return request(root + `/chapters/${identifier(a.chapterId)}/completion`, { read: a.read });
    case 'page_get':
      return request(root + `/pages/${identifier(a.pageId)}`);
    case 'page_image': {
      const p = await request<Page>(root + `/pages/${identifier(a.pageId)}`);
      return (
        prefix + root + `/pages/${p.id}/image?translated=${!!a.translated}&revision=${p.revision}`
      );
    }
    case 'thumbnail':
      return request(root + `/pages/${identifier(a.pageId)}/thumbnail`);
    case 'enqueue':
      return request(root + '/translate', { pageIds: a.pageIds, priority: a.priority ?? null });
    case 'translation_batch_preview':
      return request(
        root + '/batch' + (a.chapterId ? `?chapterId=${identifier(a.chapterId)}` : ''),
      );
    case 'translation_batch_submit':
      return request(root + '/batch', { chapterId: a.chapterId, pages: a.pages, mode: a.mode });
    case 'restart_enqueue':
    case 'prepare_enqueue':
      return request(
        root +
          `/pages/${identifier(a.pageId)}/${command === 'restart_enqueue' ? 'restart' : 'prepare'}`,
        { expected: a.expected, replace: a.replace },
      );
    case 'action_readiness':
      return request(root + '/availability', {
        pageId: a.page?.id || null,
        actions: a.actions || ['translate', 'prepare', 'edit'],
      });
    case 'edit_page': {
      const m = regionMutation(a.page, a.base, a.expected);
      return request(
        root +
          `/pages/${identifier(a.page.id)}/regions/${encodeURIComponent(m.regionId)}/${m.action}`,
        m.body,
      );
    }
    default:
      throw hostedMessage('Configure this feature on the PC.');
  }
}
export function nextJobs(offset: number) {
  return request<JobsBatch & { history: { offset: number; total: number; more: boolean } }>(
    `/jobs?offset=${offset}&limit=50`,
  );
}
