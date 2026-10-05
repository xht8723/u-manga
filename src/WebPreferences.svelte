<script lang="ts">
  import { untrack } from 'svelte';
  import Modal from './Modal.svelte';
  import { t, locale, setLanguage, uiError, tr, type UiText } from './i18n';
  import { applyTheme } from './appearance';
  import { signOut } from './hosted';
  import type { Settings } from './types';
  let {
    value,
    onclose,
    onapply,
  }: {
    value: Settings;
    onclose: () => void;
    onapply: (settings: Settings, base: Settings) => Promise<unknown>;
  } = $props();
  const base = untrack(() => structuredClone($state.snapshot(value)));
  let draft = $state(structuredClone(base)),
    busy = $state(false),
    error = $state<UiText>('');
  function preview() {
    setLanguage(draft.uiLanguage, navigator.language);
    applyTheme(draft.appearance);
  }
  function cancel() {
    setLanguage(value.uiLanguage, navigator.language);
    applyTheme(value.appearance);
    onclose();
  }
  async function save() {
    const captured = structuredClone($state.snapshot(draft));
    busy = true;
    error = '';
    try {
      await onapply(captured, base);
      onclose();
    } catch (e) {
      error = uiError(e);
    } finally {
      busy = false;
    }
  }
  async function logout() {
    busy = true;
    error = '';
    try {
      await signOut();
      cancel();
    } catch (e) {
      error = uiError(e);
    } finally {
      busy = false;
    }
  }
</script>

<Modal
  title={t('host.browserSettings', $locale)}
  onclose={cancel}
  closeDisabled={busy}
  className="browser-settings"
>
  <p>{t('host.browserLocal', $locale)}</p>
  <label
    >{t('host.language', $locale)}<select
      disabled={busy}
      bind:value={draft.uiLanguage}
      onchange={preview}
      ><option value="system">{t('host.system', $locale)}</option><option value="en">English</option
      ><option value="zh-Hans">简体中文</option></select
    ></label
  >
  <label
    >{t('host.appearance', $locale)}<select
      disabled={busy}
      bind:value={draft.appearance.active}
      onchange={preview}
      ><option value="day">{t('host.day', $locale)}</option><option value="night"
        >{t('host.night', $locale)}</option
      ></select
    ></label
  >
  <label
    >{t('host.layout', $locale)}<select disabled={busy} bind:value={draft.reader.layout}
      ><option value="continuous">{t('host.continuous', $locale)}</option><option value="paged"
        >{t('host.paged', $locale)}</option
      ></select
    ></label
  >
  <p class="host-hint">{t('host.configurePC', $locale)}</p>
  <div class="browser-session">
    <button class="sign-out" disabled={busy} onclick={logout}>{t('host.signOut', $locale)}</button>
  </div>
  {#if error}<p role="alert">{tr(error, $locale)}</p>{/if}
  {#snippet footer()}<button disabled={busy} onclick={cancel}>{t('host.cancel', $locale)}</button
    ><button class="primary" disabled={busy} onclick={save}>{t('host.apply', $locale)}</button
    >{/snippet}
</Modal>

<style>
  .browser-session {
    padding-top: 12px;
    border-top: 1px solid var(--border);
  }
  .sign-out {
    background: transparent;
    border-color: transparent;
    color: var(--muted);
    padding-inline: 0;
  }
  .sign-out:hover {
    color: var(--text);
  }
</style>
