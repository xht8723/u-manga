<script lang="ts" generics="T extends ChoiceValue">
  import { randomUuid } from './browser-id';
  import { t, tr, locale } from './i18n';
  import { onMount, tick } from 'svelte';
  import { ChevronDown, Check } from 'lucide-svelte';
  import { matchingChoices, nextChoice, type ChoiceValue, type ChoiceOption } from './choices';
  import { hosted } from './runtime-mode';
  import { observeViewport, placeOverlay, visibleViewport } from './viewport';
  let {
    value = $bindable(),
    options,
    label,
    placeholder = '',
    editable = false,
    disabled = false,
    oninput,
    oncommit,
  }: {
    value: T;
    options: ChoiceOption<T>[];
    label: string;
    placeholder?: string;
    editable?: boolean;
    disabled?: boolean;
    oninput?: (value: T) => void;
    oncommit?: (value: T) => void;
  } = $props();
  const id = `choice-${randomUuid()}`;
  let root: HTMLSpanElement;
  let control = $state<HTMLInputElement | HTMLButtonElement>();
  let popup = $state<HTMLDivElement>();
  let opened = $state(false),
    query = $state(''),
    filtering = $state(false),
    active = $state(-1);
  let scroll = $state(0),
    rowHeight = $state(38),
    listHeight = $state(300);
  let left = $state(0),
    top = $state(0),
    width = $state(200);
  let dirty = false,
    typeahead = '',
    lastKey = 0;
  let displayed = $derived(
    options.map((o) => ({
      ...o,
      label: o.literal ? o.label : tr(o.label, $locale),
      description: o.description ? tr(o.description, $locale) : undefined,
    })),
  );
  let filtered = $derived(filtering ? matchingChoices(displayed, query) : displayed);
  let selectedLabel = $derived(
    displayed.find((o) => o.value === value)?.label ?? String(value ?? ''),
  );
  let start = $derived(Math.max(0, Math.floor(scroll / rowHeight) - 2));
  let end = $derived(Math.min(filtered.length, start + Math.ceil(listHeight / rowHeight) + 5));
  function blocked() {
    return disabled || !!control?.matches(':disabled') || !!root.closest('[inert]');
  }
  function position() {
    if (!root || !opened) return;
    const r = root.getBoundingClientRect();
    const view = visibleViewport();
    if (r.bottom < view.top + 8 || r.top > view.bottom - 8) {
      close();
      return;
    }
    rowHeight = Math.max(
      hosted ? 44 : 38,
      Math.ceil(
        parseFloat(getComputedStyle(root).fontSize) *
          (options.some((o) => o.description) ? 4.8 : 2.8),
      ),
    );
    const wanted = Math.min(rowHeight * 8, Math.max(rowHeight, filtered.length * rowHeight)) + 8;
    const placed = placeOverlay(r, Math.max(r.width, 180), wanted);
    ({ left, top, width } = placed);
    listHeight = placed.maxHeight;
  }
  async function reveal(all = true) {
    if (blocked()) return;
    filtering = !all;
    opened = true;
    active = all ? filtered.findIndex((o) => o.value === value && !o.disabled) : -1;
    position();
    await tick();
    scrollActive();
  }
  function commitText() {
    if (editable && dirty) {
      dirty = false;
      oncommit?.(value);
    }
  }
  function close(commit = false) {
    opened = false;
    if (commit) commitText();
  }
  function choose(index: number) {
    const option = filtered[index];
    if (!option || option.disabled || blocked()) return;
    value = option.value;
    dirty = false;
    oninput?.(value);
    oncommit?.(value);
    close();
    control?.focus();
  }
  function scrollActive() {
    if (!popup || active < 0) return;
    const style = getComputedStyle(popup);
    const padding = parseFloat(style.paddingTop) + parseFloat(style.paddingBottom);
    if (active * rowHeight < popup.scrollTop) popup.scrollTop = active * rowHeight;
    else if ((active + 1) * rowHeight + padding > popup.scrollTop + popup.clientHeight)
      popup.scrollTop = (active + 1) * rowHeight + padding - popup.clientHeight;
    scroll = popup.scrollTop;
  }
  async function key(e: KeyboardEvent) {
    if (blocked() || e.isComposing) return;
    if (e.key === 'Escape' && opened) {
      e.preventDefault();
      e.stopPropagation();
      close();
      return;
    }
    if (e.key === 'Tab') {
      close(true);
      return;
    }
    if (e.key === 'Enter') {
      e.preventDefault();
      e.stopPropagation();
      if (!opened) await reveal();
      else if (active >= 0) choose(active);
      else close(true);
      return;
    }
    if (
      e.key === 'ArrowDown' ||
      e.key === 'ArrowUp' ||
      ((!editable || opened) && ['Home', 'End'].includes(e.key))
    ) {
      e.preventDefault();
      e.stopPropagation();
      if (!opened) await reveal();
      const forward = e.key === 'ArrowDown' || e.key === 'Home';
      const initial =
        e.key === 'Home'
          ? -1
          : e.key === 'End'
            ? filtered.length
            : active < 0 && !forward
              ? filtered.length
              : active;
      active = nextChoice(filtered, initial, forward ? 1 : -1);
      await tick();
      scrollActive();
    } else if (!editable && e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey) {
      e.preventDefault();
      if (!opened) await reveal();
      const now = Date.now();
      typeahead = now - lastKey > 700 ? e.key : typeahead + e.key;
      lastKey = now;
      active = filtered.findIndex(
        (o) => !o.disabled && o.label.toLocaleLowerCase().startsWith(typeahead.toLocaleLowerCase()),
      );
      await tick();
      scrollActive();
    }
  }
  function attachPopup(node: HTMLDivElement) {
    // A sibling overlay inside the active scrim escapes the modal body's scroll clipping.
    const host = root.closest('.scrim') || document.body;
    host.appendChild(node);
    return {
      destroy() {
        node.remove();
      },
    };
  }
  onMount(() => {
    const outside = (e: PointerEvent) => {
      if (!root.contains(e.target as Node) && !popup?.contains(e.target as Node)) close(true);
    };
    const reposition = (e: Event) => {
      if (e.target !== popup) position();
    };
    document.addEventListener('pointerdown', outside, true);
    document.addEventListener('scroll', reposition, true);
    const stopViewport = observeViewport(position);
    const resize = new ResizeObserver(position);
    resize.observe(root);
    const observer = new MutationObserver(() => {
      if (blocked()) close();
    });
    for (let el: Element | null = root; el; el = el.parentElement)
      observer.observe(el, { attributes: true, attributeFilter: ['disabled', 'inert'] });
    return () => {
      resize.disconnect();
      observer.disconnect();
      document.removeEventListener('pointerdown', outside, true);
      document.removeEventListener('scroll', reposition, true);
      stopViewport();
    };
  });
  $effect(() => {
    filtered;
    if (opened) {
      position();
      if (active >= filtered.length) active = -1;
    }
  });
</script>

<span class="combo" bind:this={root}>
  {#if editable}
    <input
      bind:this={control}
      role="combobox"
      aria-label={tr(label, $locale)}
      aria-expanded={opened}
      aria-controls={opened ? id : undefined}
      aria-autocomplete="list"
      aria-activedescendant={opened && active >= start && active < end
        ? `${id}-${active}`
        : undefined}
      value={String(value ?? '')}
      placeholder={tr(placeholder, $locale)}
      {disabled}
      autocomplete="off"
      spellcheck="false"
      onkeydown={key}
      onblur={commitText}
      onclick={() => {
        if (!opened) void reveal();
      }}
      oninput={(e) => {
        value = e.currentTarget.value as T;
        query = e.currentTarget.value;
        dirty = true;
        oninput?.(value);
        active = -1;
        scroll = 0;
        if (popup) popup.scrollTop = 0;
        void reveal(false);
      }}
    />
    <button
      type="button"
      class="combo-arrow"
      tabindex="-1"
      {disabled}
      aria-label={tr(`Show ${tr(label, $locale)} options`, $locale)}
      title={tr(
        opened ? `Hide ${tr(label, $locale)} options` : `Show ${tr(label, $locale)} options`,
        $locale,
      )}
      onpointerdown={(e) => e.preventDefault()}
      onclick={() => {
        control?.focus();
        if (opened && !filtering) close();
        else void reveal();
      }}><ChevronDown size={16} /></button
    >
  {:else}
    <button
      bind:this={control}
      type="button"
      role="combobox"
      aria-label={tr(label, $locale)}
      title={`${tr(label, $locale)}: ${selectedLabel || tr(placeholder, $locale)}`}
      aria-expanded={opened}
      aria-controls={opened ? id : undefined}
      aria-haspopup="listbox"
      aria-activedescendant={opened && active >= start && active < end
        ? `${id}-${active}`
        : undefined}
      {disabled}
      onkeydown={key}
      onclick={() => (opened ? close() : void reveal())}
      ><span>{selectedLabel || tr(placeholder, $locale)}</span><ChevronDown size={16} /></button
    >
  {/if}
</span>
{#if opened}
  <div
    use:attachPopup
    bind:this={popup}
    class="combo-popup"
    {id}
    role="listbox"
    aria-label={tr(`${tr(label, $locale)} options`, $locale)}
    style:left={`${left}px`}
    style:top={`${top}px`}
    style:width={`${width}px`}
    style:max-height={`${listHeight}px`}
    onscroll={(e) => (scroll = e.currentTarget.scrollTop)}
  >
    {#if filtered.length}
      <div class="combo-options-space" style:height={`${filtered.length * rowHeight}px`}>
        <div class="combo-options" style:transform={`translateY(${start * rowHeight}px)`}>
          {#each filtered.slice(start, end) as option, i (`${typeof option.value}:${option.value}`)}
            <button
              type="button"
              role="option"
              tabindex="-1"
              id={`${id}-${start + i}`}
              class:highlighted={active === start + i}
              aria-selected={option.value === value}
              aria-disabled={!!option.disabled}
              disabled={option.disabled}
              title={option.description ? `${option.label} — ${option.description}` : option.label}
              style:height={`${rowHeight}px`}
              style:min-height={`${rowHeight}px`}
              style:max-height={`${rowHeight}px`}
              onpointerdown={(e) => e.preventDefault()}
              onpointermove={() => (active = start + i)}
              onclick={() => choose(start + i)}
              ><span class="combo-option-copy"
                ><span>{option.label}</span>{#if option.description}<small
                    >{tr(option.description, $locale)}</small
                  >{/if}</span
              >{#if option.value === value}<Check size={15} />{/if}</button
            >
          {/each}
        </div>
      </div>
    {:else}<div class="combo-empty">
        {tr(editable ? 'No suggestions · keep typing a custom value' : 'No options', $locale)}
      </div>{/if}
  </div>
{/if}
