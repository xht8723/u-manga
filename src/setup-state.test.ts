import { describe, it, expect } from 'vitest';
import { settingsDefaults } from './book-state';
import { setupSteps, stepIssues, newProvider, testSignature } from './setup-state';
import { previewReadiness, scope } from './setup-preview';
describe('guided setup contracts', () => {
  it('Next is governed by current-step prerequisites, not future downloads', () => {
    const s = settingsDefaults();
    expect(previewReadiness(s, [], []).advancement.library.ready).toBe(false);
    s.libraryDirectory = 'Fixture';
    const r = previewReadiness(s, [], []);
    expect(r.advancement.library.ready).toBe(true);
    expect(r.advancement.pipeline.ready).toBe(true);
    expect(r.advancement.services.ready).toBe(true);
    expect(r.advancement.models.ready).toBe(false);
    s.translation.mode = 'vision';
    s.translation.cleanup.method = 'solid';
    expect(previewReadiness(s, [], []).advancement.services.ready).toBe(false);
  });
  it('thinking invalidates only service test configuration, not stored profile selection', () => {
    const s = settingsDefaults(),
      p = newProvider();
    s.providers = [p];
    s.translation.providerId = p.id;
    const key = testSignature(s);
    p.thinking = true;
    expect(testSignature(s)).not.toBe(key);
    expect(s.translation.providerId).toBe(p.id);
  });
  it('defaults to Local OCR, Manga OCR, and Manga LaMa, with explicit Vision/Solid skipping Downloads', () => {
    const s = settingsDefaults();
    expect(s.translation.cleanup).toEqual({
      method: 'manga_lama',
      strategy: 'automatic',
      device: 'directml',
    });
    expect(setupSteps('vision')).toEqual(['library', 'pipeline', 'services', 'models', 'review']);
    expect(
      previewReadiness(s, [], [])
        .packs.filter((p) => p.required)
        .map((p) => p.id),
    ).toEqual(['rtdetr_int8', 'manga_ocr_onnx', 'inpaint_manga_lama']);
    expect(s.translation.mode).toBe('local');
    expect(s.translation.ocr).toBe('manga');
    s.translation.cleanup.method = 'solid';
    expect(setupSteps('vision', 'solid')).toEqual(['library', 'pipeline', 'services', 'review']);
    expect(setupSteps('local', 'solid')).toContain('models');
  });
  it('cannot finish a fresh setup and does not silently select a service', () => {
    const s = settingsDefaults();
    expect(s.setup.completed).toBe(false);
    expect(s.translation.providerId).toBe('');
    const r = previewReadiness(s, [], []);
    expect(r.ready).toBe(false);
    expect(stepIssues(r, 'pipeline', 'vision').some((i) => i.section === 'library')).toBe(true);
    expect(setupSteps('vision', 'solid')).not.toContain('models');
    expect(setupSteps('local')).toContain('models');
  });
  it('vision can finish without a download folder or optional test', () => {
    const s = settingsDefaults(),
      p = newProvider();
    p.model = 'explicit';
    p.vision = true;
    s.providers = [p];
    s.translation.providerId = p.id;
    s.translation.mode = 'vision';
    s.libraryDirectory = 'Fixture';
    s.translation.cleanup.method = 'solid';
    const r = previewReadiness(s, [scope(p)], []);
    expect(r.ready).toBe(true);
    expect(stepIssues(r, 'review', 'vision', true)).toEqual([]);
    expect(r.packs.filter((p) => p.required).map((p) => p.id)).toEqual(['rtdetr_int8']);
  });
  it('requires language-specific OCR plus the line detector, and respects credential host changes', () => {
    const s = settingsDefaults(),
      p = newProvider();
    p.service = 'google';
    s.providers = [p];
    s.translation.providerId = p.id;
    s.libraryDirectory = 'Fixture';
    s.translation.cleanup.method = 'solid';
    s.translation.mode = 'local';
    s.translation.sourceLanguage = 'ko';
    expect(previewReadiness(s, [scope(p)], []).issues.some((i) => i.code === 'ocr_language')).toBe(
      true,
    );
    expect(stepIssues(previewReadiness(s, [scope(p)], []), 'services', 'local')).toHaveLength(1);
    expect(stepIssues(previewReadiness(s, [scope(p)], []), 'models', 'local')).toHaveLength(1);
    s.translation.ocr = 'pp';
    s.libraryDirectory = 'Models';
    const r = previewReadiness(s, [scope(p)], []);
    expect(r.packs.filter((p) => p.required).map((p) => p.id)).toEqual([
      'rtdetr_int8',
      'pp_det',
      'pp_korean',
    ]);
    expect(stepIssues(r, 'models', 'local')).toEqual([]);
    expect(stepIssues(r, 'review', 'local').length).toBe(2);
    expect(
      previewReadiness(s, [scope(p)], ['Models/models:pp_det', 'Models/models:pp_korean']).ready,
    ).toBe(true);
    p.service = 'llm';
    const old = scope(p);
    p.endpoint = 'https://another.invalid';
    expect(previewReadiness(s, [old], []).credentialStored).toBe(false);
  });
  it('invalidates test results on service or pipeline changes, not appearance', () => {
    const s = settingsDefaults();
    const key = testSignature(s);
    s.appearance.active = 'night';
    expect(testSignature(s)).toBe(key);
    s.translation.cleanup.method = 'manga_lama';
    expect(testSignature(s)).toBe(key);
    s.translation.mode = 'vision';
    expect(testSignature(s)).not.toBe(key);
  });
});
