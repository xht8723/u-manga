<script lang="ts">
  import { onMount } from 'svelte';
  import { Server, Copy } from 'lucide-svelte';
  import Modal from './Modal.svelte';
  import { call } from './bridge';
  import { t, tr, locale, uiError, type UiText } from './i18n';
  import type { HostStatus } from './types';
  let { onclose, onstatus }: { onclose: () => void; onstatus: (status: HostStatus) => void } =
    $props();
  let status = $state<HostStatus | null>(null),
    port = $state(8080),
    password = $state(''),
    pending = $state(false),
    error = $state<UiText>(''),
    url = $state(''),
    qr = $state(''),
    copied = $state(false);
  let live = true,
    sequence = 0,
    statusSequence = 0;
  async function refresh() {
    const current = ++statusSequence;
    let next: HostStatus;
    try {
      next = await call('host_status');
    } catch (error) {
      if (live && current === statusSequence) throw error;
      else return;
    }
    if (!live || current !== statusSequence) return;
    if (!status) port = next.port;
    status = next;
    onstatus(next);
    if (!next.urls.includes(url)) url = next.urls[0] || '';
  }
  async function act(start: boolean) {
    if (pending) return;
    const current = ++statusSequence;
    pending = true;
    error = '';
    try {
      const next = start
        ? await call('host_start', { port, password: password || null })
        : await call('host_stop');
      if (!live || current !== statusSequence) return;
      status = next;
      password = '';
      onstatus(status);
      url = status.urls[0] || '';
    } catch (e) {
      if (live && current === statusSequence) error = uiError(e);
    } finally {
      if (live && current === statusSequence) pending = false;
    }
  }
  $effect(() => {
    const selected = url,
      current = ++sequence;
    copied = false;
    if (!selected) {
      qr = '';
      return;
    }
    void call('host_qr', { url: selected })
      .then((svg) => {
        if (current === sequence && live) qr = svg;
      })
      .catch(() => {});
  });
  async function copy() {
    try {
      await navigator.clipboard.writeText(url);
      copied = true;
    } catch {
      document.querySelector<HTMLInputElement>('.host-url input')?.select();
    }
  }
  onMount(() => {
    void refresh().catch((e) => (error = uiError(e)));
    const timer = setInterval(() => {
      if (!pending) void refresh().catch(() => {});
    }, 5000);
    return () => {
      live = false;
      clearInterval(timer);
    };
  });
</script>

<Modal
  title={t('host.title', $locale)}
  {onclose}
  closeDisabled={pending}
  className="hosting-modal"
  stretch
>
  <div class="host-running">
    <Server size={18} /><strong
      >{t(status?.running ? 'host.running' : 'host.stopped', $locale)}</strong
    ><span class="spacer"></span>{#if status?.running}<small
        >{t('host.sessions', $locale, { count: status.sessions })}</small
      >{/if}
  </div>
  <fieldset disabled={pending || !!status?.running} class="host-config">
    <label
      >{t('host.port', $locale)}<input type="number" min="1" max="65535" bind:value={port} /></label
    >
    <label
      >{t('host.password', $locale)}<input
        type="password"
        autocomplete="new-password"
        bind:value={password}
        placeholder={t(
          status?.passwordConfigured ? 'host.savedPassword' : 'host.passwordMinimum',
          $locale,
        )}
      /></label
    >
  </fieldset>
  <div class="host-connection">
    <div class="host-connection-fields">
      <label
        >{t('host.address', $locale)}<select bind:value={url}
          >{#each status?.urls || [] as item}<option value={item}>{item}</option>{/each}</select
        ></label
      >
      <div class="host-url">
        <input readonly value={url} aria-label={t('host.address', $locale)} /><button
          onclick={copy}
          disabled={!url}
          title={t('host.copy', $locale)}
          ><Copy size={16} />{t(copied ? 'host.copied' : 'host.copy', $locale)}</button
        >
      </div>
    </div>
    {#if qr}<img
        class="host-qr"
        src={`data:image/svg+xml;base64,${btoa(unescape(encodeURIComponent(qr)))}`}
        alt={t('host.qr', $locale)}
        width="144"
        height="144"
      />{/if}
  </div>
  <p class="host-security">{t('host.security', $locale)}</p>
  <div class="host-status-message" role="status">
    {#if error || status?.error}<span class="error-text"
        >{tr(error || status?.error || '', $locale)}</span
      >{/if}
  </div>
  {#snippet footer()}
    <button onclick={onclose} disabled={pending}>{t('host.close', $locale)}</button>
    <span class="spacer"></span>
    <button
      class:primary={!status?.running}
      disabled={pending ||
        !status ||
        (!status.running &&
          (!Number.isInteger(port) ||
            port < 1 ||
            port > 65535 ||
            (!status.passwordConfigured && password.length < 8) ||
            (!!password && password.length < 8)))}
      onclick={() => act(!status?.running)}
      >{t(pending ? 'host.working' : status?.running ? 'host.stop' : 'host.start', $locale)}</button
    >
  {/snippet}
</Modal>
