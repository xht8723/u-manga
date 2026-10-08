import { describe, expect, it } from 'vitest';
import { groupJobErrors } from './jobs-feedback';

describe('bulk job feedback', () => {
  it('groups separately deserialized errors without losing diagnostics or identities', () => {
    const message = { key: 'conflict', args: {}, fallback: 'Page changed', detail: 'revision' };
    const errors = Array.from({ length: 172 }, (_, id) => ({ id: String(id), message: structuredClone(message) }));
    errors.push({ id: 'other', message: { ...message, detail: 'storage' } });
    const groups = groupJobErrors(errors);
    expect(groups).toHaveLength(2);
    expect(groups[0].ids).toHaveLength(172);
    expect(groups[1].ids).toEqual(['other']);
  });
  it('does not count the same job twice in one group', () => {
    expect(groupJobErrors([{ id: 'a', message: 'Failed' }, { id: 'a', message: 'Failed' }])[0].ids).toEqual(['a']);
  });
});
