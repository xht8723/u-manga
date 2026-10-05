<script lang="ts">
  import { tr, locale, uiError, type UiText } from './i18n';
  import { call } from './bridge';
  import Modal from './Modal.svelte';
  import ComboBox from './ComboBox.svelte';
  import { instructionTasks, instructionVariables, templateIssue } from './instructions';
  import type {
    Provider,
    TranslationSettings,
    InstructionTask,
    InstructionOverrides,
    InstructionPreview,
  } from './types';
  let {
    provider = $bindable(),
    settings,
    readonly = false,
    onwarnings,
  }: {
    provider: Provider;
    settings: TranslationSettings;
    readonly?: boolean;
    onwarnings?: (messages: UiText[]) => void;
  } = $props();
  let task = $state<InstructionTask>('textTranslation');
  let defaults = $state<InstructionOverrides | null>(null);
  let defaultsLanguage = $state('');
  let loadError = $state<UiText>('');
  let preview = $state<InstructionPreview | null>(null);
  let previewBusy = $state(false);
  let previewError = $state<UiText>('');
  let copied = $state(false);
  let opened = $state(false);
  let generation = 0;
  let previewGeneration = 0;
  let previewButton = $state<HTMLButtonElement>();
  let text = $derived(
    provider.instructions[task] ??
      (defaultsLanguage === settings.sourceLanguage ? defaults?.[task] : null) ??
      '',
  );
  let ready = $derived(
    provider.instructions[task] !== null || defaultsLanguage === settings.sourceLanguage,
  );
  let issue = $derived(templateIssue(text));
  $effect(() => {
    const message = opened && (issue || loadError || previewError);
    onwarnings?.(message ? [message] : []);
    return () => onwarnings?.([]);
  });
  $effect(() => {
    const sourceLanguage = settings.sourceLanguage;
    if (!opened) return;
    const token = ++generation;
    loadError = '';
    void call('prompt_defaults', { sourceLanguage })
      .then((value) => {
        if (token === generation) {
          defaults = value;
          defaultsLanguage = sourceLanguage;
        }
      })
      .catch((error) => {
        if (token === generation) loadError = uiError(error);
      });
    return () => {
      generation++;
    };
  });
  $effect(() => {
    provider.id;
    provider.service;
    provider.protocol;
    task;
    settings.sourceLanguage;
    settings.targetLanguage;
    provider.instructions.textTranslation;
    provider.instructions.visionTranslation;
    provider.instructions.glossaryDetection;
    provider.simpleTranslation;
    provider.instructions.simpleTextTranslation;
    previewGeneration++;
    preview = null;
    previewBusy = false;
    previewError = '';
    return () => {
      previewGeneration++;
    };
  });
  async function showPreview() {
    const token = ++previewGeneration;
    previewBusy = true;
    previewError = '';
    copied = false;
    try {
      const result = await call('prompt_preview', {
        profile: $state.snapshot(provider),
        settings: $state.snapshot(settings),
        task,
      });
      if (token === previewGeneration) preview = result;
    } catch (error) {
      if (token === previewGeneration) previewError = uiError(error);
    } finally {
      if (token === previewGeneration) previewBusy = false;
    }
  }
  async function copy() {
    if (!preview) return;
    try {
      await navigator.clipboard.writeText(
        `SYSTEM\n${preview.system}\n\nUSER (synthetic sample)\n${preview.user}`,
      );
      copied = true;
    } catch (error) {
      previewError = uiError(error);
    }
  }
</script>

<details class="setup-advanced instruction-section" bind:open={opened}>
  <summary>{tr('Prompts', $locale)}</summary>
  <div class="instruction-editor">
    {#if readonly}<p class="hint">{tr('Managed by the selected service profile.', $locale)}</p>{/if}
    <div class="instruction-heading">
      <ComboBox label={tr('Prompt task', $locale)} bind:value={task} options={instructionTasks} />
      <span class="instruction-badge"
        >{tr(provider.instructions[task] === null ? 'Default' : 'Custom', $locale)}</span
      >
    </div>
    <label class="instruction-label"
      >{tr('System prompt', $locale)}
      <textarea
        class="instruction-text"
        aria-label={tr('System prompt', $locale)}
        value={text}
        {readonly}
        disabled={!ready}
        spellcheck="false"
        oninput={(event) => {
          provider.instructions[task] = event.currentTarget.value;
        }}></textarea>
    </label>
    {#if !readonly}<section class="instruction-help">
        <h4>{tr('Language placeholders', $locale)}</h4>
        <p class="hint">
          {tr(
            'Edit the full system prompt. Empty means no system instructions. Limit: 16 KiB UTF-8.',
            $locale,
          )}
        </p>
        <div class="variables">
          {#each instructionVariables as variable}<code>{`{{${variable}}}`}</code>{/each}
        </div>
        <p class="hint">
          {tr(
            'Language placeholders insert names (Japanese) or codes (ja) from the processing settings.',
            $locale,
          )}
        </p>
      </section>{/if}
    <div class="instruction-actions">
      {#if !readonly}<button
          class="secondary"
          disabled={provider.instructions[task] === null || !ready}
          onclick={() => {
            provider.instructions[task] = null;
          }}>{tr('Restore default', $locale)}</button
        >{/if}
      <button
        bind:this={previewButton}
        class="secondary"
        disabled={!ready || !!issue || previewBusy}
        onclick={showPreview}>{tr(previewBusy ? 'Preparing preview…' : 'Preview', $locale)}</button
      >
    </div>
    <div class="instruction-status" aria-live="polite">
      {#if issue || loadError || previewError}<span class="error"
          >{tr(issue || loadError || previewError, $locale)}</span
        >{/if}
    </div>
  </div>
</details>
{#if preview}
  <Modal
    title={tr('Prompt preview', $locale)}
    onclose={() => {
      preview = null;
    }}
    className="instruction-preview"
    wide
    portal
    returnFocus={previewButton}
  >
    <section>
      <h3>{tr('System prompt', $locale)}</h3>
      <pre>{preview.system || tr('(Empty system prompt)', $locale)}</pre>
    </section>
    <section>
      <h3>{tr('User prompt example', $locale)}</h3>
      <pre>{preview.user}</pre>
    </section>
    {#if previewError}<p class="error">{tr(previewError, $locale)}</p>{/if}
    {#snippet footer()}<button onclick={copy}>{tr(copied ? 'Copied' : 'Copy', $locale)}</button
      ><button
        class="primary"
        onclick={() => {
          preview = null;
        }}>{tr('Close', $locale)}</button
      >{/snippet}
  </Modal>
{/if}

<style>
  .instruction-editor {
    display: grid;
    gap: 12px;
    padding-top: 14px;
  }
  .hint {
    margin: 0;
    color: var(--muted);
    font-size: 0.9em;
    line-height: 1.5;
  }
  .instruction-heading {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
  }
  .instruction-heading :global(.combo) {
    flex: 1;
    min-width: min(100%, 180px);
  }
  .instruction-badge {
    background: var(--surface-alt, var(--surface));
    color: var(--muted);
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 3px 10px;
    font-size: 0.8em;
  }
  .instruction-label {
    display: grid;
    gap: 8px;
  }
  .instruction-text {
    width: 100%;
    box-sizing: border-box;
    height: 200px;
    min-height: 150px;
    max-height: 340px;
    resize: vertical;
    line-height: 1.55;
    font-size: max(1em, 16px);
    font-weight: 400;
  }
  .instruction-help h4 {
    margin: 0 0 6px;
    font-size: 0.9em;
  }
  .variables {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin-top: 8px;
  }
  code {
    font-size: 0.8em;
    overflow-wrap: anywhere;
  }
  .instruction-actions {
    display: flex;
    gap: 10px;
    flex-wrap: wrap;
    justify-content: flex-end;
  }
  .instruction-actions button {
    min-height: 44px;
  }
  .instruction-status {
    min-height: 1.4em;
    font-size: 0.85em;
    overflow-wrap: anywhere;
  }
  pre {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    font: inherit;
    line-height: 1.5;
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 14px;
    margin: 12px 0 20px;
  }
  h3 {
    font-size: 1em;
    margin: 20px 0 8px;
  }
  :global(.instruction-preview) {
    width: min(860px, 96vw);
    height: min(740px, 88dvh);
  }
</style>
