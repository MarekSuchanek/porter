<script lang="ts">
  import Icon from './Icon.svelte';
  import { app } from '../lib/state.svelte';

  let element = $state<HTMLDivElement>();
  let position = $state({ x: 0, y: 0 });

  // Keep the menu inside the window.
  $effect(() => {
    const menu = app.menu;
    if (!menu || !element) return;
    const { width, height } = element.getBoundingClientRect();
    position = {
      x: Math.min(menu.x, window.innerWidth - width - 6),
      y: Math.min(menu.y, window.innerHeight - height - 6),
    };
  });

  function close() {
    app.menu = null;
  }
</script>

<svelte:window onblur={close} onresize={close} />

{#if app.menu}
  <div class="backdrop" role="presentation" onmousedown={close} oncontextmenu={(e) => { e.preventDefault(); close(); }}></div>
  <div class="menu" role="menu" bind:this={element} style:left="{position.x}px" style:top="{position.y}px">
    {#each app.menu.items as entry, i (i)}
      {#if entry === 'separator'}
        <div class="sep" role="separator"></div>
      {:else}
        <button
          role="menuitem"
          class:danger={entry.danger}
          disabled={entry.disabled}
          onclick={() => {
            close();
            entry.run();
          }}
        >
          <span class="icon">{#if entry.icon}<Icon name={entry.icon} size={14} />{/if}</span>
          <span class="label">{entry.label}</span>
          {#if entry.keys}<span class="keys">{entry.keys}</span>{/if}
        </button>
      {/if}
    {/each}
  </div>
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 40;
  }
  .menu {
    position: fixed;
    z-index: 41;
    min-width: 210px;
    padding: 5px;
    border-radius: 9px;
    background: var(--raised);
    border: 1px solid var(--border);
    box-shadow: var(--shadow);
  }
  button {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    height: 26px;
    padding: 0 8px;
    border-radius: 5px;
    text-align: left;
  }
  button:hover:not(:disabled) {
    background: var(--accent);
    color: var(--accent-text);
  }
  button:hover:not(:disabled) .keys {
    color: inherit;
  }
  button:disabled {
    opacity: 0.4;
  }
  .danger {
    color: var(--danger);
  }
  .icon {
    width: 14px;
    display: grid;
  }
  .label {
    flex: 1;
  }
  .keys {
    font-size: 11px;
    color: var(--faint);
  }
  .sep {
    height: 1px;
    margin: 4px 6px;
    background: var(--border);
  }
</style>
