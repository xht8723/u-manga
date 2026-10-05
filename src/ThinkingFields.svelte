<script lang="ts">
  import { t, tr, locale } from './i18n';
  import { call } from './bridge';
  import { latestCheck, type CheckState } from './latest-check';
  import { untrack } from 'svelte';
  import ToggleSwitch from './ToggleSwitch.svelte';
  import type { Provider, ThinkingPolicy } from './types';
  let {
    provider = $bindable(),
    readonly = false,
    refreshKey = 0,
    onchange = () => {},
  }: {
    provider: Provider;
    readonly?: boolean;
    refreshKey?: number;
    onchange?: () => void;
  } = $props();
  let result = $state<CheckState<ThinkingPolicy>>({
    value: null,
    fresh: false,
    pending: true,
    showActivity: false,
    error: '',
  });
  let checked = $state('');
  let key = $derived(
    JSON.stringify([provider.service, provider.protocol, provider.endpoint, provider.model]),
  );
  const loader = latestCheck(
    (profile: Provider) => call('thinking_capability', { profile }),
    (state) => (result = state),
  );
  $effect(() => {
    refreshKey;
    checked = key;
    return loader.start(untrack(() => $state.snapshot(provider)));
  });
  let fresh = $derived(checked === key && result.fresh);
  let policy = $derived(fresh ? result.value : null);
  let capabilityState = $derived(policy?.state || 'managed');
  let explanation = $derived(
    readonly
      ? 'Managed by the selected service profile.'
      : !fresh
        ? result.error
          ? 'Model support is unavailable. Your Thinking preference still applies.'
          : 'Checking model support. You can change Thinking now.'
        : capabilityState === 'fixed_on'
          ? 'This model reports required thinking. Off may be rejected or ignored.'
          : capabilityState === 'fixed_off'
            ? 'This model reports no thinking support. Explicit Thinking settings may be rejected or ignored.'
            : capabilityState === 'managed'
              ? 'Model support is unknown. Unsupported Thinking choices may be rejected or ignored.'
              : 'Off requests no thinking. On uses the lowest known thinking effort.',
  );
</script>

{#if provider.service === 'llm'}<div class="thinking-field">
    <ToggleSwitch
      label={t('m_a20d12c5e9c4', $locale)}
      checked={provider.thinking}
      disabled={readonly}
      focusableWhenDisabled
      title={tr(explanation, $locale)}
      onchange={(value) => {
        provider.thinking = value;
        provider.thinkingPolicy = null;
        onchange();
      }}
    />
    <small>{tr(explanation, $locale)}</small>
  </div>{/if}

<style>
  .thinking-field {
    margin: 12px 0;
    min-height: 72px;
  }
  .thinking-field > small {
    display: block;
    color: var(--muted);
    margin: 4px 10px;
  }
</style>
