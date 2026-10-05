import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { regionMutation, hostedCall, applyBrowserPreferences, request } from './hosted';
import { settingsDefaults } from './book-state';
import { emptyGlossary } from './glossary';
import type { Page } from './types';
const page = (): Page => ({
  id: '00000000-0000-4000-8000-000000000001',
  number: 0,
  name: 'page',
  source: { path: '', kind: 'image', entry: null, index: 0 },
  width: 400,
  height: 600,
  fingerprint: '',
  revision: 3,
  regions: [
    {
      id: 'dialogue',
      bbox: [10, 20, 50, 80],
      bubble: null,
      kind: 'text',
      score: 1,
      source: '原文',
      target: '译文',
      direction: 'auto',
      style: {
        font: null,
        size: null,
        color: '#202020',
        fill: '#fff',
        lineGap: 0.2,
        outlineEnabled: false,
        outlineWidthPercent: 8,
        outlineColor: '#ffffff',
      },
      allowFill: false,
      overlayOnly: false,
      prepared: false,
      review: null,
    },
  ],
  rendered: null,
  background: null,
  cleanup: null,
  status: 'edited',
  error: null,
});
describe('hosted transport boundary', () => {
  it('reads glossary terms explicitly and saves with an opaque book ID and revision guard', async () => {
    const glossary = {
      ...emptyGlossary(),
      enabled: true,
      entries: [{ source: 'アリス', target: '爱丽丝' }],
      revision: 7,
    };
    const fetch = vi.fn().mockResolvedValue({ ok: true, json: async () => ({ glossary }) });
    vi.stubGlobal('fetch', fetch);
    const path = 'book:00000000-0000-4000-8000-000000000002';
    await hostedCall('book_open', { path });
    await hostedCall('book_open', { path, glossary: true });
    await hostedCall('book_glossary_save', { path, expected: 7, glossary });
    expect(fetch.mock.calls.map((c) => c[0])).toEqual([
      '/api/v1/books/00000000-0000-4000-8000-000000000002',
      '/api/v1/books/00000000-0000-4000-8000-000000000002/glossary',
      '/api/v1/books/00000000-0000-4000-8000-000000000002/glossary',
    ]);
    expect(JSON.parse(fetch.mock.calls[2][1].body)).toEqual({ expected: 7, glossary });
    await expect(
      hostedCall('book_glossary_save', { path: 'C:/private/book.umanga', expected: 7, glossary }),
    ).rejects.toMatchObject({ fallback: expect.stringContaining('Book is no longer available') });
    expect(fetch).toHaveBeenCalledTimes(3);
  });
  it('parses imported glossary files in the browser without sending them to the server', async () => {
    const fetch = vi.fn();
    vi.stubGlobal('fetch', fetch);
    expect(
      await hostedCall('glossary_parse', {
        text: 'source\ttarget\nアリス\t爱丽丝\n',
        format: 'tsv',
      }),
    ).toEqual([{ source: 'アリス', target: '爱丽丝' }]);
    await expect(
      hostedCall('glossary_parse', { text: '"unclosed', format: 'csv' }),
    ).rejects.toThrow();
    expect(fetch).not.toHaveBeenCalled();
  });
  it('retains saved outlines for text edits and rejects hosted typography changes', () => {
    const base = page();
    Object.assign(base.regions[0].style, {
      outlineEnabled: true,
      outlineWidthPercent: 15,
      outlineColor: '#ffee99',
    });
    const draft = structuredClone(base);
    draft.regions[0].target = 'corrected';
    expect(regionMutation(draft, base, 3)).toMatchObject({
      body: { expected: 3, source: '原文', target: 'corrected' },
    });
    expect(draft.regions[0].style).toEqual(base.regions[0].style);
    draft.regions[0].style.outlineEnabled = false;
    expect(() => regionMutation(draft, base, 3)).toThrow();
  });
  beforeEach(() => {
    vi.stubGlobal('window', { dispatchEvent: vi.fn() });
    vi.stubGlobal('localStorage', { getItem: vi.fn().mockReturnValue(null), setItem: vi.fn() });
  });
  afterEach(() => vi.unstubAllGlobals());
  it('sends only identity, expected revision, and one region text change', async () => {
    const base = page(),
      draft = structuredClone(base);
    draft.regions[0].target = '新译文';
    const fetch = vi.fn().mockResolvedValue({ ok: true, json: async () => draft });
    vi.stubGlobal('fetch', fetch);
    await hostedCall('edit_page', {
      path: 'book:00000000-0000-4000-8000-000000000002',
      page: draft,
      base,
      expected: 3,
    });
    expect(fetch.mock.calls[0][0]).toBe(
      '/api/v1/books/00000000-0000-4000-8000-000000000002/pages/00000000-0000-4000-8000-000000000001/regions/dialogue/text',
    );
    expect(JSON.parse(fetch.mock.calls[0][1].body)).toEqual({
      expected: 3,
      source: '原文',
      target: '新译文',
    });
  });
  it('rejects geometry, additions, and multiple region mutations', () => {
    const base = page(),
      draft = structuredClone(base);
    draft.regions[0].bbox = [1, 2, 3, 4];
    expect(() => regionMutation(draft, base, 3)).toThrow();
    draft.regions = structuredClone(base.regions);
    draft.regions.push({ ...draft.regions[0], id: 'other' });
    expect(() => regionMutation(draft, base, 3)).toThrow();
  });
  it('deletes only the selected identity', () => {
    const base = page(),
      draft = structuredClone(base);
    draft.regions = [];
    expect(regionMutation(draft, base, 3)).toEqual({
      regionId: 'dialogue',
      action: 'delete',
      body: { expected: 3 },
    });
    expect(() => regionMutation(draft, base, 4)).toThrow();
  });
  it('never falls back to preview or repeats an unavailable mutation', async () => {
    const fetch = vi.fn().mockRejectedValue(new Error('disconnected'));
    vi.stubGlobal('fetch', fetch);
    await expect(request('/jobs/control', { action: 'stop' })).rejects.toMatchObject({
      fallback: expect.stringContaining('Connection lost'),
    });
    expect(fetch).toHaveBeenCalledTimes(1);
    await expect(hostedCall('secret_set', { value: 'secret' })).rejects.toMatchObject({
      fallback: 'Configure this feature on the PC.',
    });
    expect(fetch).toHaveBeenCalledTimes(1);
  });
  it('keeps structured authentication errors and drafts outside transport', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({
        ok: false,
        status: 401,
        json: async () => ({ error: { key: 'host.signIn', args: {}, fallback: 'Sign in' } }),
      }),
    );
    await expect(request('/books')).rejects.toEqual({
      key: 'host.signIn',
      args: {},
      fallback: 'Sign in',
    });
    expect(window.dispatchEvent).toHaveBeenCalled();
  });
  it('rejects an interrupted successful body without acknowledging or repeating a save', async () => {
    const fetch = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => {
        throw Error('body interrupted');
      },
    });
    vi.stubGlobal('fetch', fetch);
    await expect(
      request('/page/text', { expected: 3, source: '原文', target: '译文' }),
    ).rejects.toMatchObject({ fallback: expect.stringContaining('drafts are retained') });
    expect(fetch).toHaveBeenCalledTimes(1);
    expect(window.dispatchEvent).toHaveBeenCalled();
  });
  it('browser preferences cannot change translation configuration', async () => {
    const saved = settingsDefaults();
    saved.translation.ocr = 'ppocr';
    saved.reader.zoom = 160;
    vi.mocked(localStorage.getItem).mockReturnValue(
      JSON.stringify({
        uiLanguage: 'zh-Hans',
        reader: { layout: 'paged', zoom: 110 },
        translation: { ocr: 'manga' },
        libraryDirectory: 'attack',
        providers: [{ endpoint: 'attack' }],
      }),
    );
    const result = applyBrowserPreferences(saved);
    expect(result.translation).toEqual(saved.translation);
    expect(result.libraryDirectory).toBe(saved.libraryDirectory);
    expect(result.providers).toEqual(saved.providers);
    expect(result.reader).toEqual({ layout: 'paged', zoom: 110 });
    expect(saved.reader.zoom).toBe(160);
    await hostedCall('preferences', { settings: result });
    expect(localStorage.setItem).toHaveBeenCalled();
    const stored = JSON.parse(vi.mocked(localStorage.setItem).mock.calls[0][1]);
    expect(stored.translation).toBeUndefined();
    expect(stored.providers).toBeUndefined();
  });
  it('merges unrelated browser saves and rejects stale conflicting changes', async () => {
    let stored: string | null = null;
    vi.mocked(localStorage.getItem).mockImplementation(() => stored);
    vi.mocked(localStorage.setItem).mockImplementation((_key, value) => {
      stored = value;
    });
    const base = applyBrowserPreferences(settingsDefaults());
    const language = structuredClone(base),
      view = structuredClone(base);
    language.uiLanguage = 'zh-Hans';
    view.libraryView = 'list';
    await hostedCall('preferences', { settings: language, base });
    const saved = await hostedCall('preferences', { settings: view, base });
    expect(saved.uiLanguage).toBe('zh-Hans');
    expect(saved.libraryView).toBe('list');
    const conflict = structuredClone(base);
    conflict.uiLanguage = 'en';
    const before = stored;
    await expect(hostedCall('preferences', { settings: conflict, base })).rejects.toThrow(
      'Settings changed elsewhere',
    );
    expect(stored).toBe(before);
    vi.mocked(localStorage.setItem).mockImplementation(() => {
      throw Error('storage unavailable');
    });
    const failed = structuredClone(saved);
    failed.libraryView = 'grid';
    await expect(hostedCall('preferences', { settings: failed, base: saved })).rejects.toThrow(
      'storage unavailable',
    );
    expect(stored).toBe(before);
    expect(applyBrowserPreferences(base)).toEqual(saved);
  });
  it('uses server-owned availability and never sends page/configuration objects', async () => {
    const fetch = vi.fn().mockResolvedValue({ ok: true, json: async () => ({}) });
    vi.stubGlobal('fetch', fetch);
    await hostedCall('action_readiness', {
      path: 'book:00000000-0000-4000-8000-000000000002',
      page: page(),
      actions: ['edit'],
      settings: { providers: ['private'] },
    });
    expect(JSON.parse(fetch.mock.calls[0][1].body)).toEqual({
      pageId: page().id,
      actions: ['edit'],
    });
  });
});
