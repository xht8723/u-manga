import { expect, it } from 'vitest';
import { newProvider } from './setup-state';
import { hostedThinking } from './thinking';

it('shares exact hosted capabilities without applying them to sibling/custom models', () => {
  const p = newProvider();
  p.protocol = 'responses';
  for (const [model, state, on, off] of [
    ['gpt-5.2', 'switchable', 'low', 'none'],
    ['gpt-5.2-2025-12-11', 'switchable', 'low', 'none'],
    ['gpt-5.2-pro', 'fixed_on', 'medium', null],
    ['gpt-5-pro', 'fixed_on', 'high', null],
    ['gpt-5.2-codex', 'managed', null, null],
    ['custom/gpt-5.2', 'managed', null, null],
  ] as const) {
    p.model = model;
    expect(hostedThinking(p)).toEqual({ state, on, off });
  }
});
