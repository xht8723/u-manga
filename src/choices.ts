export type ChoiceValue = string | number | boolean;
export type ChoiceOption<T extends ChoiceValue = ChoiceValue> = {
  value: T;
  label: string;
  description?: string;
  disabled?: boolean;
  /** User content, font family names and provider/model identifiers are not UI messages. */
  literal?: boolean;
};

export function matchingChoices<T extends ChoiceValue>(options: ChoiceOption<T>[], query: string) {
  const text = query.toLocaleLowerCase();
  return options.filter((o) => o.label.toLocaleLowerCase().includes(text));
}

export function nextChoice<T extends ChoiceValue>(
  options: ChoiceOption<T>[],
  index: number,
  direction: number,
) {
  for (let i = index + direction; i >= 0 && i < options.length; i += direction) {
    if (!options[i].disabled) return i;
  }
  return index;
}
