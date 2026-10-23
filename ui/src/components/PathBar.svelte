<script lang="ts">
  import { tick } from 'svelte';
  import { app, type Pane } from '../lib/state.svelte';
  import { dragOver, drop } from '../lib/dnd';

  let { pane, active }: { pane: Pane; active: boolean } = $props();

  let editing = $state(false);
  let value = $state('');
  let input = $state<HTMLInputElement>();
  let dropTarget = $state<string | null>(null);
  let seen = app.editPath;

  // ⌘L / "Go to location…" edits the active pane's path.
  $effect(() => {
    const request = app.editPath;
    if (request !== seen) {
      seen = request;
      if (active) void edit();
    }
  });

  async function edit() {
    value = pane.location;
    editing = true;
    await tick();
    input?.focus();
    input?.select();
  }

  function onKey(e: KeyboardEvent) {
    e.stopPropagation();
    if (e.key === 'Enter') {
      const target = value.trim();
      editing = false;
      if (target && target !== pane.location) void pane.open(target);
    } else if (e.key === 'Escape') {
      editing = false;
    }
  }
</script>

{#if editing}
  <input
    class="field path"
    bind:this={input}
    bind:value
    spellcheck="false"
    autocomplete="off"
    aria-label="Location"
    onkeydown={onKey}
    onblur={() => (editing = false)}
  />
{:else}
  <div class="crumbs" role="navigation" aria-label="Path" ondblclick={edit}>
    {#each pane.listing?.crumbs ?? [] as crumb, i (crumb.location)}
      {#if i > 0}<span class="sep">›</span>{/if}
      <button
        class="crumb"
        class:last={i === (pane.listing?.crumbs.length ?? 0) - 1}
        class:drop={dropTarget === crumb.location}
        title={crumb.location}
        onclick={() => pane.open(crumb.location)}
        ondragover={(e) => dragOver(e, crumb.location) && (dropTarget = crumb.location)}
        ondragleave={() => (dropTarget = null)}
        ondrop={(e) => {
          dropTarget = null;
          drop(e, crumb.location);
        }}
      >
        {crumb.name}
      </button>
    {/each}
    <button class="spacer" aria-label="Edit location" onclick={edit}></button>
  </div>
{/if}

<style>
  .path {
    flex: 1;
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: 12px;
  }
  .crumbs {
    /* Content-sized basis so the filter box next to it gives way first. */
    flex: 1 1 auto;
    min-width: 72px;
    display: flex;
    align-items: center;
    height: 28px;
    padding: 0 4px;
    border-radius: 6px;
    border: 1px solid transparent;
    overflow: hidden;
  }
  .crumbs:hover {
    border-color: var(--border);
  }
  .crumb {
    flex: 0 1 auto;
    min-width: 24px;
    max-width: 180px;
    padding: 2px 6px;
    border-radius: 5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--muted);
  }
  .crumb.last {
    flex-shrink: 0;
    color: var(--text);
    font-weight: 600;
  }
  .crumb:hover,
  .crumb.drop {
    background: var(--hover);
    color: var(--text);
  }
  .crumb.drop {
    outline: 2px solid var(--accent);
  }
  .sep {
    flex: none;
    color: var(--faint);
    padding: 0 1px;
  }
  .spacer {
    flex: 1;
    align-self: stretch;
    min-width: 20px;
    cursor: text;
  }
</style>
