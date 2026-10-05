import { uiError, type UiText } from './i18n';
export type CheckState<T> = {
  value: T | null;
  fresh: boolean;
  pending: boolean;
  showActivity: boolean;
  error: UiText;
};

// Retain displayed results while invalidating their authority immediately.
export function latestCheck<Input, Output>(
  run: (input: Input) => Promise<Output>,
  publish: (state: CheckState<Output>) => void,
) {
  let value: Output | null = null;
  let generation = 0;
  let debounce: ReturnType<typeof setTimeout>;
  let activity: ReturnType<typeof setTimeout>;
  function cancel() {
    generation++;
    clearTimeout(debounce);
    clearTimeout(activity);
  }
  return {
    cancel,
    start(input: Input) {
      cancel();
      const request = generation;
      const update = (fresh: boolean, pending: boolean, showActivity = false, error: UiText = '') =>
        publish({ value, fresh, pending, showActivity, error });
      update(false, true);
      debounce = setTimeout(() => {
        activity = setTimeout(() => {
          if (request === generation) update(false, true, true);
        }, 200);
        void run(input).then(
          (next) => {
            if (request !== generation) return;
            clearTimeout(activity);
            value = next;
            update(true, false);
          },
          (error) => {
            if (request !== generation) return;
            clearTimeout(activity);
            update(false, false, false, uiError(error));
          },
        );
      }, 180);
      return cancel;
    },
  };
}
