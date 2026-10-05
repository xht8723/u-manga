import { describe, it, expect } from 'vitest';
import { matchingChoices, nextChoice, type ChoiceOption } from './choices';
describe('combobox choices', () => {
  it('retains distinct boolean, numeric and string values while skipping disabled choices', () => {
    const options: ChoiceOption[] = [
      { value: false, label: 'Paged' },
      { value: true, label: 'Continuous', disabled: true },
      { value: 0, label: 'Region zero' },
      { value: '0', label: 'Model zero' },
    ];
    expect(nextChoice(options, 0, 1)).toBe(2);
    expect(nextChoice(options, 2, -1)).toBe(0);
    expect(nextChoice(options, 3, 1)).toBe(3);
    expect(matchingChoices(options, 'zero').map((o) => o.value)).toEqual([0, '0']);
    expect(matchingChoices(options, 'PAGED')[0].value).toBe(false);
  });
  it('filters large catalogs without mutating them or inventing a custom selection', () => {
    const options = Array.from({ length: 2000 }, (_, i) => ({
      value: `model-${i}`,
      label: `Model ${i}`,
    }));
    expect(matchingChoices(options, 'Model 1999')).toEqual([options[1999]]);
    expect(matchingChoices(options, 'custom model')).toEqual([]);
    expect(options).toHaveLength(2000);
    expect(nextChoice([], -1, 1)).toBe(-1);
  });
});
