import { describe, it, expect } from 'vitest';
import {
  previewDefaults,
  previewInstructions,
  templateIssue,
  instructionIssue,
  expandInstructions,
} from './instructions';
import fixtures from '../resources/instruction-previews.json';
import { settingsDefaults } from './book-state';
import { newProvider, readinessSignature, testSignature } from './setup-state';
import { mergePreferences } from './preferences';
import type { InstructionTask } from './types';
describe('editable instructions', () => {
  it('keeps simple mode independent with separate system and user previews', () => {
    const settings = settingsDefaults(),
      p = newProvider();
    settings.providers = [p];
    settings.translation.providerId = p.id;
    expect(p.simpleTranslation).toBe(true);
    const savedOff = { ...p, simpleTranslation: false };
    expect(JSON.parse(JSON.stringify(savedOff)).simpleTranslation).toBe(false);
    expect(p.instructions.simpleTextTranslation).toBeNull();
    const ready = readinessSignature(settings),
      tested = testSignature(settings);
    p.simpleTranslation = true;
    p.instructions.simpleTextTranslation = 'Use {{target_language}} naturally.';
    expect(readinessSignature(settings)).toBe(ready);
    expect(testSignature(settings)).not.toBe(tested);
    p.instructions.textTranslation = 'Structured guidance only';
    const preview = previewInstructions(p, settings.translation, 'simpleTextTranslation');
    expect(preview.system).toBe('Use Simplified Chinese naturally.');
    expect(preview.schema).toBeNull();
    expect(preview.user).not.toContain('Use Simplified Chinese naturally.');
    expect(preview.user).toContain('Current texts:\n[1] 123\n[2] 456\n[3] 789');
    expect(preview.user).not.toMatch(/Region ID|Structured guidance|regions|source echo/);
    p.instructions.simpleTextTranslation = '';
    expect(previewInstructions(p, settings.translation, 'simpleTextTranslation').system).toBe('');
    expect(
      previewInstructions(p, settings.translation, 'simpleTextTranslation').user.startsWith(
        'Glossary:',
      ),
    ).toBe(true);
    p.simpleTranslation = false;
    expect(p.instructions.simpleTextTranslation).toBe('');
    p.instructions.simpleTextTranslation = '{{unknown}}';
    expect(instructionIssue(p)).toContain('unknown');
  });
  it('shows existing glossary as quoted pairs without object wrappers', () => {
    const p = newProvider();
    p.protocol = 'ollama';
    for (const task of ['textTranslation', 'visionTranslation'] as const) {
      const result = previewInstructions(p, settingsDefaults().translation, task);
      expect(result.user).toContain('Glossary:');
      expect(result.user).toContain('\n"Sample name": "Preferred name"\n');
      expect(result.user).not.toContain('[{"source":');
      expect(result.schema).not.toBeNull();
    }
  });
  it('shows only current pairs for default, custom and empty glossary templates', () => {
    const profile = newProvider(),
      settings = settingsDefaults().translation;
    settings.glossaryEnabled = true;
    settings.glossary = [{ source: 'EXISTING-ONLY-SENTINEL', target: 'DO-NOT-SEND' }];
    for (const template of [null, '{{target_language}} only', '']) {
      profile.instructions.glossaryDetection = template;
      const result = previewInstructions(profile, settings, 'glossaryDetection');
      expect(result.user).toBe('Current source texts:\n\n[1] Source: "123"');
      expect(result.schema).toBeNull();
      expect(result.system).toBe(
        template === null
          ? expandInstructions(previewDefaults('ja').glossaryDetection!, settings)
          : template === ''
            ? ''
            : 'Simplified Chinese only',
      );
    }
  });
  it('keeps translation defaults concise without weakening the protocol schema', () => {
    const p = newProvider();
    p.protocol = 'ollama';
    for (const task of ['textTranslation', 'visionTranslation'] as const) {
      const result = previewInstructions(p, settingsDefaults().translation, task);
      expect(result.system.match(/Japanese \(ja\)/g)).toHaveLength(1);
      expect(result.system.match(/Simplified Chinese \(zh-Hans\)/g)).toHaveLength(1);
      expect(result.system).not.toContain('Response JSON Schema');
      expect(result.user).toContain(
        'Use previous dialogue as context; translate only current text.',
      );
      expect(result.schema).toBeTruthy();
      expect(result.system.length).toBeLessThan(1300);
    }
  });
  it('matches native synthetic previews and schemas', () => {
    for (const row of fixtures) {
      const settings = structuredClone(settingsDefaults().translation);
      settings.sourceLanguage = row.source;
      const profile = newProvider();
      profile.protocol = row.protocol;
      expect(previewInstructions(profile, settings, row.task as InstructionTask)).toEqual(
        row.preview,
      );
    }
  });
  it('replaces defaults and preserves intentionally empty guidance', () => {
    const profile = newProvider();
    const settings = settingsDefaults().translation;
    profile.instructions.glossaryDetection = 'Ordinary nouns are welcome.';
    let result = previewInstructions(profile, settings, 'glossaryDetection');
    expect(result.system).toContain('Ordinary nouns are welcome.');
    expect(result.system).not.toContain('Selection examples:');
    expect(result.system).not.toContain('Ordinary titles');
    expect(result.system).toBe('Ordinary nouns are welcome.');
    profile.instructions.textTranslation = '';
    result = previewInstructions(profile, settings, 'textTranslation');
    expect(result.prompt).toBe('');
    expect(result.system).toBe('');
  });
  it('validates UTF-8 limits and substitutes only documented placeholders once', () => {
    expect(templateIssue('汉'.repeat(6000))).toContain('16 KiB');
    expect(templateIssue('a'.repeat(16384))).toBe('');
    expect(templateIssue('{{unknown}}')).toContain('unknown');
    expect(templateIssue('{"a":{"b":"中文"}}')).toBe('');
    expect(
      expandInstructions('{{source_language_code}}', {
        ...settingsDefaults().translation,
        sourceLanguage: '{{target_language}}',
      }),
    ).toBe('{{target_language}}');
  });
  it('invalidates service tests without changing dependency readiness', () => {
    const settings = settingsDefaults();
    const p = newProvider();
    settings.providers = [p];
    settings.translation.providerId = p.id;
    const ready = readinessSignature(settings);
    const tested = testSignature(settings);
    p.instructions.textTranslation = 'More concise.';
    expect(readinessSignature(settings)).toBe(ready);
    expect(testSignature(settings)).not.toBe(tested);
    p.instructions.glossaryDetection = '{{bad}}';
    expect(instructionIssue(p)).toContain('bad');
  });
  it('preserves unrelated settings and rejects conflicting profile commits', () => {
    const base = settingsDefaults();
    base.providers = [newProvider()];
    const draft = structuredClone(base),
      current = structuredClone(base);
    draft.providers[0].instructions.textTranslation = 'Draft';
    current.libraryView = 'list';
    const result = mergePreferences(base, draft, current);
    expect(result.libraryView).toBe('list');
    expect(result.providers[0].instructions.textTranslation).toBe('Draft');
    current.providers[0].instructions.glossaryDetection = 'Concurrent';
    expect(() => mergePreferences(base, draft, current)).toThrow();
  });
  it('uses the supplied Chinese glossary default for every source language', () => {
    expect(previewDefaults('ja').glossaryDetection).toBe(
      '识别并提取地名，人名，称号，组织名，并把他们翻译到{{target_language}}。\n只提取词汇，不提取整句。\n格式：\n[数字] 原词:翻译\n例子：\n[1] タルク王国:塔尔克王国\n[2] アリス:爱丽丝',
    );
    expect(previewDefaults('ja').glossaryDetection).toBe(previewDefaults('en').glossaryDetection);
    expect(previewDefaults('en').glossaryDetection).not.toContain('ケーラ');
    const profile = newProvider();
    profile.instructions.textTranslation = '{{target_language}}';
    expect(
      previewInstructions(profile, settingsDefaults().translation, 'textTranslation').prompt,
    ).toBe('Simplified Chinese');
  });
});
