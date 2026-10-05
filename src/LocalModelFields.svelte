<script lang="ts">
  import { t, tr, locale } from './i18n';
  import { onMount } from 'svelte';
  import { listen } from '@tauri-apps/api/event';
  import { call, native } from './bridge';
  import { requiredModelIds } from './setup-state';
  import { modelDirectory, type ModelStorage } from './model-storage';
  import type { Settings, ModelPack, Readiness } from './types';
  let {
    settings = $bindable(),
    models,
    report,
    bookMode = false,
    guided = false,
    onchange,
    onerror,
  }: {
    settings: Settings;
    models: ModelPack[];
    report: Readiness | null;
    bookMode?: boolean;
    guided?: boolean;
    onchange: () => void;
    onerror: (e: unknown) => void;
  } = $props();
  let optional = $state(false);
  let storage = $state<ModelStorage | null>(null);
  let applied = $derived(!!storage?.library && storage.library === settings.libraryDirectory);
  let downloads = $state<Record<string, number>>({}),
    verifying = $state('');
  let bundled = $derived(models.filter((p) => p.distribution === 'bundled'));
  let required = $derived(
    models.filter(
      (p) =>
        p.distribution !== 'bundled' &&
        requiredModelIds(settings.translation, models).includes(p.id),
    ),
  );
  let others = $derived(
    models.filter((p) => p.distribution !== 'bundled' && !required.includes(p)),
  );
  const size = (p: ModelPack) => (p.files.reduce((n, f) => n + f.bytes, 0) / 2 ** 20).toFixed(2);
  async function download(id: string) {
    downloads[id] = 0;
    try {
      await call('model_download', { id, library: settings.libraryDirectory });
      onchange();
    } catch (e) {
      onerror(e);
    } finally {
      delete downloads[id];
    }
  }
  async function verify(id: string) {
    verifying = id;
    try {
      const valid = await call('model_verify', { id, library: settings.libraryDirectory });
      onchange();
      if (!valid)
        onerror('Files are missing or damaged. Use Download / resume to repair this pack.');
    } catch (e) {
      onerror(e);
    } finally {
      verifying = '';
    }
  }
  onMount(() => {
    let live = true;
    void call('model_storage')
      .then((result) => {
        if (live) storage = result;
      })
      .catch(onerror);
    void call('model_downloads').then((ids) => {
      if (live) for (const id of ids) downloads[id] = 0;
    });
    if (!native)
      return () => {
        live = false;
      };
    const progress = listen<{ id: string; done: number; total: number }>(
      'download',
      ({ payload: p }) => {
        if (live) downloads[p.id] = p.done / p.total;
      },
    );
    const ended = listen<{ id: string }>('download-ended', ({ payload: p }) => {
      if (live) {
        delete downloads[p.id];
        onchange();
      }
    });
    return () => {
      live = false;
      void progress.then((f) => f());
      void ended.then((f) => f());
    };
  });
</script>

{#each bundled as pack}{@render packRow(pack)}{/each}
{#if !required.length}<p class="notice">{t('m_80641bb87480', $locale)}</p>{/if}
{#if bookMode}<p class="setup-help">{t('m_2328f5180413', $locale)}</p>{/if}
{#if required.length > 0 || optional}
  {#if !bookMode && storage && !applied}<p class="notice">
      {settings.libraryDirectory ? t('m_af18057a76a5', $locale) : t('m_3fcd49d5e906', $locale)}
    </p>{/if}
{/if}
{#if required.length}<h4>{t('m_3f3aa36939dc', $locale)}</h4>{/if}
{#each required as pack}{@render packRow(pack)}{/each}
{#if !bookMode && !guided}<button
    aria-expanded={optional}
    title={t('m_381108831e10', $locale)}
    onclick={() => (optional = !optional)}
    >{optional ? t('m_ac20a57bfde0', $locale) : t('m_0df6f1cad36c', $locale)}
    {t('m_fb3c6704c3f7', $locale)}</button
  >{/if}
{#if optional}{#each others as pack}{@render packRow(pack)}{/each}{/if}
{#if required.length > 0 || optional}
  <div class="model-location">
    <span>{t('m_8c82985e951b', $locale)}</span><code
      >{modelDirectory(settings) || t('m_503f201129a0', $locale)}</code
    >
  </div>
{/if}
{#snippet packRow(pack: ModelPack)}
  {@const status =
    report?.packs.find((p) => p.id === pack.id)?.status || (report ? 'missing' : 'checking')}
  <article class="model-card" data-model-id={pack.id}>
    <div>
      <b>{pack.distribution === 'bundled' ? t('m_f48b2b212887', $locale) : pack.name}</b>
      <p>
        {pack.distribution === 'bundled'
          ? t('m_717fa48f48d1', $locale)
          : tr(pack.description, $locale)}
      </p>
      <small
        >{#if pack.distribution === 'bundled'}{pack.name} INT8 ·
        {/if}{size(pack)} MiB · {pack.kind === 'inpainting'
          ? t('m_9f1b237b5dc3', $locale)
          : pack.languages.join(', ') || t('m_e1e7f595229a', $locale)} · {pack.license}</small
      >
    </div>
    {#if pack.distribution === 'bundled'}<span class="tag">{t('m_ba829a98b799', $locale)}</span
      >{/if}
    <span class="tag"
      >{status === 'checking'
        ? t('m_ec963ffc911b', $locale)
        : status === 'verified'
          ? t('m_4f7838402f37', $locale)
          : status === 'corrupt'
            ? t('m_4ed3689d9d35', $locale)
            : status === 'downloaded'
              ? t('m_3dddf7d417a7', $locale)
              : t('m_5569a4e0a4ce', $locale)}</span
    >
    {#if !bookMode && pack.distribution !== 'bundled'}{#if pack.id in downloads}<progress
          aria-label={tr(`${pack.name} download`, $locale)}
          value={downloads[pack.id]}
          max="1"
        ></progress><button
          title={t('m_468912ed4752', $locale)}
          onclick={() => call('model_cancel', { id: pack.id }).catch(onerror)}
          >{t('m_de5b23e69cd1', $locale)}</button
        >
      {:else}<button
          disabled={!applied || !!verifying}
          title={tr(`Verify ${pack.name} checksums`, $locale)}
          onclick={() => verify(pack.id)}
          >{verifying === pack.id
            ? t('m_ec963ffc911b', $locale)
            : t('m_eea2745e2867', $locale)}</button
        >{#if status !== 'verified'}<button
            disabled={!applied}
            title={tr(`Download ${pack.name} into the library’s models directory`, $locale)}
            onclick={() => download(pack.id)}>{t('m_dea8d3f1225a', $locale)}</button
          >{/if}{/if}{/if}
  </article>
{/snippet}
