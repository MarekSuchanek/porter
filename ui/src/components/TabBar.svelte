<script lang="ts">
  import Icon from './Icon.svelte';
  import { app, prefs } from '../lib/state.svelte';
  import { keysFor } from '../lib/commands';

  function onAuxClick(e: MouseEvent, id: number) {
    if (e.button === 1) app.closeTab(id);
  }
</script>

<div class="bar">
  <div class="tabs" role="tablist">
    {#each app.tabs as tab (tab.id)}
      <div
        class="tab"
        class:active={tab.id === app.activeTabId}
        role="tab"
        tabindex="-1"
        aria-selected={tab.id === app.activeTabId}
        title={tab.active?.location}
        onclick={() => (app.activeTabId = tab.id)}
        onkeydown={() => {}}
        onauxclick={(e) => onAuxClick(e, tab.id)}
      >
        {#if tab.dual}<Icon name="split" size={13} />{/if}
        <span class="title">{tab.dual ? tab.panes.map((p) => p.title).join(' · ') : tab.active?.title}</span>
        {#if app.tabs.length > 1}
          <button class="close" title="Close tab ({keysFor('close-tab')})" onclick={(e) => { e.stopPropagation(); app.closeTab(tab.id); }}>
            <Icon name="close" size={12} />
          </button>
        {/if}
      </div>
    {/each}
    <button class="tool" title="New tab ({keysFor('new-tab')})" onclick={() => app.newTab()}><Icon name="plus" /></button>
  </div>

  <div class="actions">
    <button class="tool" class:on={app.tab?.dual} title="Dual pane ({keysFor('toggle-dual')})" onclick={() => app.tab?.toggleDual()}>
      <Icon name="split" />
    </button>
    <button class="tool" class:on={prefs.showHidden} title="Hidden files ({keysFor('toggle-hidden')})" onclick={() => (prefs.showHidden = !prefs.showHidden)}>
      <Icon name="eye" />
    </button>
    <button class="tool" class:on={prefs.inspector} title="Inspector ({keysFor('toggle-inspector')})" onclick={() => (prefs.inspector = !prefs.inspector)}>
      <Icon name="info" />
    </button>
    <button class="tool" title="Commands ({keysFor('palette')})" onclick={() => (app.palette = true)}>
      <Icon name="command" />
    </button>
  </div>
</div>

<style>
  .bar {
    display: flex;
    align-items: flex-end;
    gap: 8px;
    height: 38px;
    padding: 0 8px 0 6px;
    background: var(--bg);
    border-bottom: 1px solid var(--border);
  }
  .tabs {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: flex-end;
    gap: 2px;
    overflow-x: auto;
    scrollbar-width: none;
  }
  .tabs > .tool {
    margin: 0 0 5px 2px;
  }
  .tab {
    display: flex;
    align-items: center;
    gap: 6px;
    max-width: 220px;
    min-width: 90px;
    height: 31px;
    padding: 0 6px 0 12px;
    border: 1px solid transparent;
    border-bottom: 0;
    border-radius: 8px 8px 0 0;
    color: var(--muted);
  }
  .tab:hover {
    background: var(--hover);
  }
  .tab.active {
    background: var(--panel);
    border-color: var(--border);
    color: var(--text);
    margin-bottom: -1px;
    height: 32px;
  }
  .title {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .close {
    display: grid;
    place-items: center;
    width: 18px;
    height: 18px;
    border-radius: 4px;
    opacity: 0;
  }
  .tab:hover .close,
  .tab.active .close {
    opacity: 0.7;
  }
  .close:hover {
    background: var(--hover);
    opacity: 1;
  }
  .actions {
    display: flex;
    gap: 2px;
    padding-bottom: 5px;
  }
</style>
