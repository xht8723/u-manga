import { describe, it, expect } from 'vitest';
import { settingsDefaults } from './book-state';
import { previewAction, previewReadiness } from './setup-preview';
import { activeElapsed, duration } from './job-clock';
import { jobProgress } from './jobs-state';
import type { Job, Page } from './types';
describe('manual setup and operation-specific requirements', () => {
  const configured = () => ({ ...settingsDefaults(), libraryDirectory: 'Fixture' });
  const models = ['Fixture/models:manga_ocr_onnx', 'Fixture/models:inpaint_manga_lama'];
  it('can finish Local OCR setup without any provider or credential, but cannot auto translate', () => {
    const s = configured();
    expect(previewReadiness(s, [], models)).toMatchObject({
      ready: true,
      credentialRequired: false,
    });
    expect(previewAction(s, 'prepare', [], models).ready).toBe(true);
    expect(previewAction(s, 'translate', [], models).issues.map((i) => i.code)).toContain(
      'service',
    );
    s.translation.mode = 'vision';
    expect(previewReadiness(s, [], models).ready).toBe(false);
    expect(previewAction(s, 'prepare', [], models).issues.map((i) => i.code)).toContain('mode');
  });
  it('requires Japanese for Manga OCR and only the selected engine packs', () => {
    const s = configured();
    s.translation.sourceLanguage = 'ko';
    expect(previewAction(s, 'prepare', [], models).issues.map((i) => i.code)).toContain(
      'ocr_language',
    );
    s.translation.ocr = 'pp';
    const p = previewAction(s, 'prepare', [], models);
    expect(p.issues.map((i) => i.code)).toEqual(['pack:pp_det', 'pack:pp_korean']);
  });
  it('isolates re-clean and service tests from unrelated inference/service requirements', () => {
    const s = configured();
    const page = { regions: [{ prepared: true, target: '' }] } as Page;
    expect(previewAction(s, 'cleanup', [], ['Fixture/models:inpaint_manga_lama'], page).ready).toBe(
      true,
    );
    expect(previewAction(s, 'prepare', [], []).ready).toBe(false);
    s.translation.providerId = 'o';
    s.providers = [
      {
        id: 'o',
        service: 'llm',
        protocol: 'ollama',
        endpoint: 'http://localhost:11434',
        model: 'local-text:3b',
        simpleTranslation: false, instructions: { textTranslation: null, simpleTextTranslation: null, visionTranslation: null, glossaryDetection: null },
      } as any,
    ];
    expect(previewAction(s, 'service', [], []).ready).toBe(true);
    s.providers[0].endpoint = 'http://offline:11434';
    expect(previewAction(s, 'service', [], []).ready).toBe(false);
    expect(previewAction(s, 'cleanup', [], ['Fixture/models:inpaint_manga_lama'], page).ready).toBe(
      true,
    );
  });
});
describe('active timers and preparation progress', () => {
  it('counts running/stopping only and preserves accumulated duration on resume/retry', () => {
    const j = { elapsedMs: 5000, receivedAt: 200, timerRunning: true, status: 'running' } as Job;
    expect(activeElapsed(j, 2200)).toBe(7000);
    expect(activeElapsed({ ...j, status: 'paused', stopping: true }, 2200)).toBe(7000);
    for (const status of ['queued', 'paused', 'failed', 'complete', 'cancelled'] as const)
      expect(activeElapsed({ ...j, status, timerRunning: false }, 900000)).toBe(5000);
    expect(activeElapsed({ ...j, elapsedMs: 7000, receivedAt: 900000 }, 901000)).toBe(8000);
    expect(duration(3601000)).toBe('1:00:01');
  });
  it('does not imply a translation or lettering step for preparation', () => {
    const job = {
      kind: 'preparation',
      status: 'running',
      stage: 'ocr',
      settings: settingsDefaults().translation,
    } as unknown as Job;
    expect(jobProgress(job).steps).toEqual(['loading', 'detecting', 'ocr', 'cleaning', 'saving']);
  });
});
