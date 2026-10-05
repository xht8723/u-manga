import { uiJoin } from './i18n';
import { canResumeJob } from './jobs-state';
import { onMount, untrack } from 'svelte';
import { call } from './bridge';
import { latestCheck, type CheckState } from './latest-check';
import type { ActionName, ActionReadiness, Availability, Job, Page, Settings } from './types';

function refreshedCheck<I, O>(key: () => string, input: () => I, run: (input: I) => Promise<O>) {
  // Many controls inspect the same readiness result. Serialize the complete input
  // once per reactive change, retaining immediate stale-result invalidation.
  const currentKey = $derived(key());
  let epoch = $state(0);
  let state = $state<CheckState<O>>({
    value: null,
    fresh: false,
    pending: true,
    showActivity: false,
    error: '',
  });
  let checked = $state('');
  let acceptedKey = $state<string | null>(null);
  const check = latestCheck<{ key: string; input: I }, { key: string; value: O }>(
    async (request) => ({ key: request.key, value: await run(request.input) }),
    (next) => {
      state = { ...next, value: next.value?.value ?? null };
      if (next.fresh && next.value) acceptedKey = next.value.key;
    },
  );
  $effect(() => {
    const signature = currentKey + ':' + epoch;
    checked = signature;
    return check.start({ key: currentKey, input: untrack(input) });
  });
  onMount(() => {
    const refresh = () => {
      epoch++;
    };
    window.addEventListener('focus', refresh);
    window.addEventListener('umanga-requirements-changed', refresh);
    return () => {
      window.removeEventListener('focus', refresh);
      window.removeEventListener('umanga-requirements-changed', refresh);
    };
  });
  return {
    get state() {
      return state;
    },
    get fresh() {
      return state.fresh && checked === currentKey + ':' + epoch;
    },
    get currentResult() {
      return acceptedKey === currentKey && !!state.value;
    },
  };
}
export function actionReadiness(
  get: () => {
    settings: Settings;
    path?: string;
    page?: Page | null;
    base?: Page | null;
    pageIds?: string[];
    actions?: ActionName[];
  },
) {
  const signature = () => {
    const { settings, ...args } = get();
    const translation = settings.translation;
    return JSON.stringify([
      args,
      translation,
      settings.providers.find((p) => p.id === translation.providerId),
      settings.libraryDirectory,
    ]);
  };
  const validation = refreshedCheck(
    signature,
    () => {
      const { settings: _, ...args } = get();
      return $state.snapshot(args);
    },
    (args) =>
      args.actions?.length === 0
        ? Promise.resolve({} as ActionReadiness)
        : call('action_readiness', args),
  );
  return {
    get fresh() {
      return validation.fresh;
    },
    ready(action: ActionName) {
      return validation.fresh && !!validation.state.value?.[action]?.ready;
    },
    displayReady(action: ActionName) {
      return (
        validation.currentResult &&
        !validation.state.error &&
        !!validation.state.value?.[action]?.ready
      );
    },
    displayReason(action: ActionName) {
      if (validation.state.error) return validation.state.error;
      if (!validation.currentResult) return 'Checking requirements…';
      return (
        uiJoin(validation.state.value?.[action]?.issues?.map((i) => i.message) || [], ' ') || ''
      );
    },
    reason(action: ActionName) {
      if (validation.state.error) return validation.state.error;
      if (!validation.fresh) return 'Checking requirements…';
      return (
        uiJoin(validation.state.value?.[action]?.issues?.map((i) => i.message) || [], ' ') || ''
      );
    },
    // Background checks disable actions immediately, but must not insert/remove
    // transient status rows above the current page's progress panel.
    hint(action: ActionName) {
      return (
        validation.state.error ||
        uiJoin(validation.state.value?.[action]?.issues?.map((i) => i.message) || [], ' ') ||
        ''
      );
    },
  };
}
export function jobsReadiness(get: () => Job[], settings: () => Settings) {
  const input = () => get().filter(canResumeJob);
  const validation = refreshedCheck(
    () =>
      JSON.stringify([
        input().map((j) => [j.id, j.requirementsKey, j.pageRevision, j.status]),
        settings().translation,
        settings().providers,
        settings().libraryDirectory,
      ]),
    () => ({ ids: input().map((j) => j.id) }),
    (args) =>
      args.ids.length
        ? call('jobs_readiness', args)
        : Promise.resolve({} as Record<string, Availability>),
  );
  return {
    ready(id: string) {
      return (
        input().some((job) => job.id === id) &&
        validation.fresh &&
        !!validation.state.value?.[id]?.ready
      );
    },
    reason(id: string) {
      return (
        validation.state.error ||
        (!validation.fresh
          ? 'Checking requirements…'
          : uiJoin(validation.state.value?.[id]?.issues?.map((i) => i.message) || [], ' ') || '')
      );
    },
    hint(id: string) {
      return (
        validation.state.error ||
        uiJoin(validation.state.value?.[id]?.issues?.map((i) => i.message) || [], ' ') ||
        ''
      );
    },
  };
}
