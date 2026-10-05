import catalog from '../resources/instructions.json';
import type {
  InstructionOverrides,
  InstructionTask,
  InstructionPreview,
  Provider,
  TranslationSettings,
} from './types';
export const instructionTasks: { value: InstructionTask; label: string }[] = [
  { value: 'textTranslation', label: 'Text translation' },
  { value: 'simpleTextTranslation', label: 'Simple text translation' },
  { value: 'visionTranslation', label: 'Vision translation' },
  { value: 'glossaryDetection', label: 'Glossary detection' },
];
export const instructionVariables = [
  'source_language',
  'source_language_code',
  'target_language',
  'target_language_code',
];
export function templateIssue(text: string): string {
  if (new TextEncoder().encode(text).length > 16384)
    return 'Prompts exceed the 16 KiB UTF-8 limit.';
  for (const match of text.matchAll(/\{\{([a-zA-Z0-9_]+)\}\}/g))
    if (!instructionVariables.includes(match[1])) return `Unknown prompt placeholder: ${match[1]}`;
  return '';
}
export function instructionIssue(profile?: Provider): string {
  if (!profile) return '';
  for (const { value } of instructionTasks) {
    const template = profile.instructions[value];
    const issue = template === null ? '' : templateIssue(template);
    if (issue) return issue;
  }
  return '';
}
export function validateInstructions(profiles: Provider[]) {
  for (const profile of profiles) {
    const issue = instructionIssue(profile);
    if (issue) throw Error(issue);
  }
}
export function captureInstructions(profile: Provider, source: string): Provider {
  validateInstructions([profile]);
  const captured = structuredClone(profile),
    defaults = previewDefaults(source);
  for (const { value } of instructionTasks) captured.instructions[value] ??= defaults[value];
  return captured;
}
// Browser fixtures use the same bundled native catalog; real desktop defaults come from IPC.
export function previewDefaults(_source: string): InstructionOverrides {
  return {
    simpleTextTranslation: catalog.simpleTextTranslation,
    textTranslation: catalog.textTranslation,
    visionTranslation: catalog.visionTranslation,
    glossaryDetection: catalog.glossaryDetection,
  };
}
const names: Record<string, string> = {
  ja: 'Japanese',
  'zh-Hans': 'Simplified Chinese',
  'zh-Hant': 'Traditional Chinese',
  en: 'English',
  ko: 'Korean',
  fr: 'French',
  de: 'German',
  es: 'Spanish',
  ar: 'Arabic',
  hi: 'Hindi',
  ru: 'Russian',
  th: 'Thai',
  vi: 'Vietnamese',
  ta: 'Tamil',
};
export function expandInstructions(text: string, s: TranslationSettings): string {
  const values: Record<string, string> = {
    source_language: names[s.sourceLanguage] || s.sourceLanguage,
    source_language_code: s.sourceLanguage,
    target_language: names[s.targetLanguage] || s.targetLanguage,
    target_language_code: s.targetLanguage,
  };
  return text.replace(/\{\{([a-zA-Z0-9_]+)\}\}/g, (_, key) => {
    if (!(key in values)) throw Error('Unknown prompt placeholder: ' + key);
    return values[key];
  });
}
export function previewInstructions(
  profile: Provider,
  s: TranslationSettings,
  task: InstructionTask,
): InstructionPreview {
  validateInstructions([profile]);
  const prompt = expandInstructions(
    profile.instructions[task] ?? previewDefaults(s.sourceLanguage)[task]!,
    s,
  );
  const glossary = task === 'glossaryDetection';
  const references = glossary
    ? []
    : [
        'Glossary:\nUse these preferred translations when applicable.\n"Sample name": "Preferred name"',
        'Use previous dialogue as context; translate only current text.\nPage 1:\n456',
      ];
  if (task === 'simpleTextTranslation') {
    const user = [...references, 'Current texts:\n[1] 123\n[2] 456\n[3] 789'].join('\n\n');
    return { prompt, system: prompt, user, schema: null };
  }
  const schema =
    profile.protocol === 'ollama' && !glossary
      ? {
          type: 'object',
          additionalProperties: false,
          required: ['regions'],
          properties: {
            regions: {
              type: 'array',
              minItems: 1,
              maxItems: 1,
              items: {
                type: 'object',
                additionalProperties: false,
                required: ['id', 'source', 'target'],
                properties: {
                  id: { type: 'string', enum: ['sample-region'] },
                  source: { type: 'string' },
                  target: { type: 'string' },
                },
              },
            },
          },
        }
      : null;
  const user = [
    ...references,
    (glossary
      ? 'Current source texts:\n\n[1] Source: "123"'
      : 'Current regions to translate:\n\nRegion ID: "sample-region"\nSource: "123"') +
      (task === 'visionTranslation' ? '\n[Sample image crop / overlapping tile attachment]' : ''),
  ].join('\n\n');
  return { prompt, system: prompt, user, schema };
}
