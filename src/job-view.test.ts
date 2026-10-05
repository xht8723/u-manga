import { test, expect } from 'vitest';
import { settingsDefaults } from './book-state';
import { jobView } from './job-view';
import type { CapturedJob } from './types';
test('preview Jobs IPC does not expose captured glossary or service destinations', () => {
  const settings = settingsDefaults().translation;
  settings.glossary = [{ source:'private-name',target:'private-target' }];
  const job = { id:'job',kind:'translation',settings,provider:{ name:'Service', model:'Model', service:'llm', endpoint:'private-host' }, modelDirectory:'private-folder', glossaryCheckpoint:{ added:3,processed:['private-id'] } } as CapturedJob;
  const view = jobView(job);
  expect(view.settings).not.toHaveProperty('glossary');
  expect(view.provider).not.toHaveProperty('endpoint');
  expect(view).not.toHaveProperty('modelDirectory');
  expect(view.glossaryCheckpoint).toEqual({ added:3 });
  expect(JSON.stringify(view)).not.toMatch(/private-(name|target|host|folder|id)/);
  const before = view.requirementsKey; job.settings.glossary.push({source:'another',target:'another'});
  expect(jobView(job).requirementsKey).toBe(before);
});
