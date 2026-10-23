<script lang="ts">
  import { tick } from 'svelte';
  import Icon from './Icon.svelte';
  import PathBar from './PathBar.svelte';
  import FileList from './FileList.svelte';
  import { app, type Pane, type Tab } from '../lib/state.svelte';
  import { keysFor } from '../lib/commands';
  import { bytes, plural } from '../lib/format';
  import { dragOver, drop } from '../lib/dnd';

  let { pane, tab, index }: { pane: Pane; tab: Tab; index: number } = $props();

  const active = $derived(tab.activeIndex === index || !tab.dual);
  let filterInput = $state<HTMLInputElement>();
  let dropping = $state(false);
  let seen = app.focusFilter;

  // Typing in the list jumps into the filter field.
  $effect(() => {
    const request = app.focusFilter;
    if (request !== seen) {
      seen = request;
      if (active) void tick().then(() => filterInput?.focus());
    }
  });

  const selectedBytes = $derived(pane.selected.reduce((sum, i) => sum + (i.size ?? 0), 0));

  function onFilterKey(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      pane.filter = '';
      filterInput?.blur();
    } else if (e.key === 'ArrowDown' || e.key === 'Enter') {
      e.preventDefault();
      filterInput?.blur();
      if (e.key === 'Enter' && pane.items[0]) pane.select(pane.items[0].location);
      else pane.moveCursor(0);
      return;
    }
    e.stopPropagation();
  }
</script>

<section
  class="pane"
  class:active={tab.dual && active}
  class:dropping
  aria-label="Folder {pane.title}"
  onpointerdown={() => (tab.activeIndex = index)}
  ondragover={(e) => (dropping = dragOver(e, pane.location))}
  ondragleave={(e) => !e.currentTarget.contains(e.relatedTarget as Node) && (dropping = false)}
  ondrop={(e) => {
    dropping = false;
    drop(e, pane.location);
  }}
>
  <header>
    <div class="nav">
      <button class="tool" title="Back ({keysFor('back')})" disabled={!pane.canBack} onclick={() => pane.back()}><Icon name="back" /></button>
      <button class="tool" title="Forward ({keysFor('forward')})" disabled={!pane.canForward} onclick={() => pane.forward()}><Icon name="forward" /></button>
      <button class="tool" title="Enclosing folder ({keysFor('up')})" disabled={!pane.listing?.parent} onclick={() => pane.up()}><Icon name="up" /></button>
    </div>
    <PathBar {pane} {active} />
    <div class="search">
      <Icon name="search" size={13} />
      <input
        bind:this={filterInput}
        bind:value={pane.filter}
        placeholder="Filter"
        spellcheck="false"
        aria-label="Filter this folder"
        onkeydown={onFilterKey}
      />
    </div>
    <button class="tool" title="New folder ({keysFor('new-folder')})" disabled={!pane.listing?.capabilities.mkdir} onclick={() => app.newFolder(pane)}><Icon name="newfolder" /></button>
    <button class="tool" class:spin={pane.loading} title="Refresh ({keysFor('refresh')})" onclick={() => pane.refresh()}><Icon name="refresh" /></button>
  </header>

  {#if pane.error}
    <div class="banner" role="alert">
      <Icon name="alert" />
      <span class="selectable">{pane.error}</span>
      <button class="btn" onclick={() => pane.refresh()}>Retry</button>
      {#if pane.listing}<button class="tool" aria-label="Dismiss" onclick={() => (pane.error = null)}><Icon name="close" /></button>{/if}
    </div>
  {/if}

  <FileList {pane} {tab} {active} />

  <footer>
    {#if pane.listing}
      <span>{plural(pane.items.length, 'item')}{pane.filter ? ` of ${pane.listing.items.length}` : ''}</span>
      {#if pane.selected.length}
        <span>· {pane.selected.length} selected{selectedBytes ? ` (${bytes(selectedBytes)})` : ''}</span>
      {/if}
      {#if pane.listing.fs?.available_bytes}
        <span class="right">{bytes(pane.listing.fs.available_bytes)} free</span>
      {:else if pane.listing.fs?.fs_type}
        <span class="right">{pane.listing.fs.fs_type.toUpperCase()}</span>
      {/if}
    {/if}
  </footer>

  {#if dropping}
    <div class="drop-hint">Drop to copy here · hold ⇧ Shift to move</div>
  {/if}
</section>

<style>
  .pane {
    position: relative;
    flex: 1;
    min-width: 0;
    overflow: hidden;
    container-type: inline-size;
    display: flex;
    flex-direction: column;
    background: var(--panel);
    border-top: 2px solid transparent;
  }
  .pane.active {
    border-top-color: var(--accent);
  }
  .pane.dropping {
    box-shadow: inset 0 0 0 2px var(--accent);
  }
  header {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 8px;
    border-bottom: 1px solid var(--border);
  }
  .nav {
    display: flex;
  }
  @container (max-width: 380px) {
    .nav .tool:not(:first-child) {
      display: none;
    }
  }
  .search {
    display: flex;
    align-items: center;
    gap: 6px;
    /* Gives way to the path bar first; expands again while in use. */
    flex: 0 6 150px;
    min-width: 30px;
    transition: flex-basis 0.15s;
    height: 28px;
    padding: 0 8px;
    border-radius: 6px;
    background: var(--bg);
    color: var(--faint);
  }
  .search:focus-within {
    box-shadow: 0 0 0 2px var(--accent);
    flex-shrink: 0;
    flex-basis: 140px;
  }
  .search input {
    flex: 1;
    min-width: 0;
    border: 0;
    outline: 0;
    background: none;
  }
  .spin :global(.icon) {
    animation: spin 0.9s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .banner {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 12px;
    background: var(--danger-weak);
    color: var(--danger);
  }
  .banner span {
    flex: 1;
  }
  footer {
    display: flex;
    gap: 6px;
    height: 24px;
    align-items: center;
    padding: 0 12px;
    border-top: 1px solid var(--border);
    font-size: 11px;
    color: var(--muted);
  }
  .right {
    margin-left: auto;
  }
  .drop-hint {
    position: absolute;
    bottom: 34px;
    left: 50%;
    transform: translateX(-50%);
    padding: 6px 12px;
    border-radius: 999px;
    background: var(--accent);
    color: var(--accent-text);
    font-size: 12px;
    pointer-events: none;
    box-shadow: var(--shadow);
  }
</style>
