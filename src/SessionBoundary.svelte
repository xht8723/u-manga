<script lang="ts">
  import { onMount } from 'svelte';
  import App from './AppShell.svelte';
  import StartupError from './StartupError.svelte';
  import { hosted } from './runtime-mode';
  import { restoreSession, signIn, connectEvents, applyBrowserPreferences } from './hosted';
  import { settingsDefaults } from './book-state';
  import { applyTheme } from './appearance';
  import { locale, setLanguage, t, tr, uiError, type UiText } from './i18n';
  let mounted = $state(!hosted), login = $state(hosted), checking = $state(hosted), offline = $state(false), password = $state(''), pending = $state(false), failure = $state<UiText>('');
  let disconnect: (() => void) | undefined;
  let reconnecting = false;
  async function reconnect() {
    if (reconnecting) return; reconnecting = true;
    checking = true;
    try { await restoreSession(); mounted = true; login = false; offline = false; disconnect?.(); disconnect = connectEvents(); }
    catch (e) { login = true; failure = uiError(e); }
    finally { checking = false; reconnecting = false; }
  }
  async function submit(e: SubmitEvent) {
    e.preventDefault(); if (pending) return; pending = true; failure = '';
    try { await signIn(password); password = ''; await reconnect(); } catch (e) { failure = uiError(e); } finally { pending = false; }
  }
  onMount(() => {
    if (!hosted) return;
    document.documentElement.dataset.runtime = 'hosted';
    const viewport = () => {
      const v = window.visualViewport;
      document.documentElement.style.setProperty('--host-visible-height', `${v?.height || innerHeight}px`);
      document.documentElement.style.setProperty('--host-keyboard-inset', `${Math.max(0, innerHeight - (v?.height || innerHeight) - (v?.offsetTop || 0))}px`);
    };
    viewport(); window.visualViewport?.addEventListener('resize', viewport); window.visualViewport?.addEventListener('scroll', viewport);
    window.addEventListener('resize', viewport);
    const local = applyBrowserPreferences(settingsDefaults());
    setLanguage(local.uiLanguage, navigator.language); applyTheme(local.appearance);
    void reconnect();
    const expired = () => { login = true; disconnect?.(); };
    const lost = () => { offline = true; };
    const restored = () => { offline = false; };
    const online = () => { if (mounted && !login) void reconnect(); };
    window.addEventListener('online', online);
    window.addEventListener('umanga-sign-in-required', expired); window.addEventListener('umanga-connection-lost', lost); window.addEventListener('umanga-connection-restored', restored);
    return () => { disconnect?.(); window.visualViewport?.removeEventListener('resize', viewport); window.visualViewport?.removeEventListener('scroll', viewport); window.removeEventListener('resize', viewport); window.removeEventListener('online', online); window.removeEventListener('umanga-sign-in-required', expired); window.removeEventListener('umanga-connection-lost', lost); window.removeEventListener('umanga-connection-restored', restored); };
  });
</script>
{#if mounted}<div class="session-app" inert={login} class:session-covered={login}><svelte:boundary>
  <App sessionCovered={login || checking} />
  {#snippet failed(error)}<StartupError message={error} />{/snippet}
</svelte:boundary></div>{/if}
{#if hosted && login}
  <div class="host-login-layer">
    <form class="host-login" onsubmit={submit} aria-label={t('host.signIn', $locale)}>
      <div class="host-wordmark">U<span>—</span>MANGA</div>
      <h1>{t('host.signIn', $locale)}</h1>
      <p>{t(mounted ? 'host.expired' : 'host.loginHelp', $locale)}</p>
      <label>{t('host.password', $locale)}<input type="password" autocomplete="current-password" bind:value={password} required disabled={pending || checking} /></label>
      <div class="host-login-status" role="status">{failure ? tr(failure, $locale) : checking ? t('host.connecting', $locale) : ''}</div>
      <button class="primary" type="submit" disabled={pending || checking || !password}>{t('host.signIn', $locale)}</button>
      <small>{t('host.trustedNetwork', $locale)}</small>
    </form>
  </div>
{:else if hosted && offline}
  <div class="host-offline" role="status"><span>{t('host.offline', $locale)}</span><button onclick={reconnect}>{t('host.reconnect', $locale)}</button></div>
{/if}
