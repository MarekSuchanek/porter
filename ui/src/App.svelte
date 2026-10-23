<script lang="ts">
  import { onMount } from 'svelte';
  import Sidebar from './components/Sidebar.svelte';
  import TabBar from './components/TabBar.svelte';
  import PaneView from './components/PaneView.svelte';
  import Inspector from './components/Inspector.svelte';
  import TransferDrawer from './components/TransferDrawer.svelte';
  import Dialog from './components/Dialog.svelte';
  import ContextMenu from './components/ContextMenu.svelte';
  import CommandPalette from './components/CommandPalette.svelte';
  import Icon from './components/Icon.svelte';
  import { app, prefs, save } from './lib/state.svelte';
  import { commands, isMac, keysFor, matches } from './lib/commands';
  import { api, inTauri } from './lib/api';
  import { checkForUpdates } from './lib/updates';

  // Shortcuts that also work while typing in a text field.
  const WHILE_TYPING = new Set([
    'palette', 'new-tab', 'close-tab', 'toggle-dual', 'toggle-inspector', 'toggle-transfers', 'toggle-hidden', 'go-location',
  ]);
  const PAGE = 15;

  const panes = $derived(app.tab ? (app.tab.dual ? app.tab.panes : [app.tab.active]) : []);
  const running = $derived(app.jobs.filter((j) => !j.summary));
  const overall = $derived.by(() => {
    let done = 0;
    let total = 0;
    for (const j of running) {
      const s = j.progress?.snapshot;
      if (s && s.bytes_total) {
        done += s.bytes_done;
        total += s.bytes_total;
      }
    }
    return total ? done / total : null;
  });

  onMount(() => {
    void app.init();
    // Release builds look for updates shortly after start; dev builds never do.
    if (inTauri && import.meta.env.PROD) setTimeout(() => void checkForUpdates(false), 5000);
    void api.onMenuSelectAll(() => {
      const field = document.activeElement;
      if (field instanceof HTMLInputElement || field instanceof HTMLTextAreaElement) field.select();
      else app.pane?.selectAll();
    });
  });

  const isField = (target: EventTarget | null) =>
    target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement;

  // When the native Edit menu handles ⌘C/⌘X/⌘V instead of a keydown reaching us, the
  // page still gets clipboard events; outside text fields they mean files.
  function onClipboard(e: ClipboardEvent, action: 'copy' | 'cut' | 'paste') {
    if (isField(e.target) || app.dialog || app.palette || !app.pane) return;
    e.preventDefault();
    if (action === 'paste') void app.paste();
    else app.copyToClipboard(action, app.pane.targets());
  }

  // Remember tabs and view settings between launches.
  $effect(() => {
    save(app.session());
  });

  function onKeydown(e: KeyboardEvent) {
    if (app.dialog || app.palette) return;
    if (app.menu) {
      if (e.key === 'Escape') app.menu = null;
      return;
    }
    const target = e.target as HTMLElement | null;
    const typing = !!target && (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable);

    for (const command of commands) {
      if (!command.keys?.some((k) => matches(e, k))) continue;
      if (typing && !WHILE_TYPING.has(command.id)) return;
      e.preventDefault();
      if (!command.when || command.when()) command.run();
      return;
    }
    if (typing) return;

    const mod = isMac ? e.metaKey : e.ctrlKey;
    if (mod && /^[1-9]$/.test(e.key)) {
      const tab = app.tabs[Number(e.key) - 1];
      if (tab) app.activeTabId = tab.id;
      e.preventDefault();
      return;
    }

    const pane = app.pane;
    if (!pane) return;
    const moves: Record<string, number> = {
      ArrowDown: 1,
      ArrowUp: -1,
      PageDown: PAGE,
      PageUp: -PAGE,
      Home: -Infinity,
      End: Infinity,
    };
    if (e.key in moves && !mod && !e.altKey) {
      e.preventDefault();
      const delta = moves[e.key];
      pane.moveCursor(Number.isFinite(delta) ? delta : delta * pane.items.length, e.shiftKey);
    } else if (e.key === 'Escape') {
      if (pane.search) pane.exitSearch();
      else if (pane.filter) pane.filter = '';
      else pane.selection.clear();
    } else if (e.key.length === 1 && e.key !== ' ' && !mod && !e.altKey && !e.ctrlKey) {
      // Type to filter.
      e.preventDefault();
      pane.filter += e.key;
      app.focusFilter++;
    }
  }

  function onContextMenu(e: MouseEvent) {
    // Our own menus handle lists; suppress the webview's Reload/Inspect menu elsewhere.
    const target = e.target as HTMLElement;
    if (inTauri && target.tagName !== 'INPUT') e.preventDefault();
  }
</script>

<svelte:window onkeydown={onKeydown} oncontextmenu={onContextMenu} />
<svelte:document
  oncopy={(e) => onClipboard(e, 'copy')}
  oncut={(e) => onClipboard(e, 'cut')}
  onpaste={(e) => onClipboard(e, 'paste')}
/>

<div class="app">
  <div class="main">
    <Sidebar />
    <div class="center">
      <TabBar />
      <div class="panes">
        {#each panes as pane, i (pane)}
          {#if i > 0}<div class="divider"></div>{/if}
          <PaneView {pane} tab={app.tab} index={i} />
        {/each}
      </div>
    </div>
    {#if prefs.inspector}<Inspector />{/if}
  </div>

  {#if prefs.drawer}<TransferDrawer />{/if}

  <footer class="status">
    {#if app.clipboard}
      <span class="clip">
        <Icon name={app.clipboard.mode === 'cut' ? 'scissors' : 'clipboard'} size={12} />
        {app.clipboard.items.length} to {app.clipboard.mode === 'cut' ? 'move' : 'copy'} — paste with {keysFor('paste')}
        <button class="x" aria-label="Clear clipboard" onclick={() => (app.clipboard = null)}><Icon name="close" size={11} /></button>
      </span>
    {:else}
      <span class="tip">Press {keysFor('palette')} for all commands</span>
    {/if}
    <button class="transfers" class:busy={running.length > 0} onclick={() => (prefs.drawer = !prefs.drawer)}>
      <Icon name="transfer" size={13} />
      {#if running.length}
        {running.length} running
        {#if overall !== null}<span class="mini"><span style:width="{overall * 100}%"></span></span>{/if}
      {:else}
        Transfers
      {/if}
    </button>
  </footer>
</div>

<Dialog />
<ContextMenu />
<CommandPalette />

{#if app.toast}
  <div class="toast" class:error={app.toast.error} role="status">
    <Icon name={app.toast.error ? 'alert' : 'check'} />
    <span class="selectable">{app.toast.text}</span>
    <button class="tool" aria-label="Dismiss" onclick={() => (app.toast = null)}><Icon name="close" size={13} /></button>
  </div>
{/if}

<style>
  .app {
    display: flex;
    flex-direction: column;
    height: 100%;
  }
  .main {
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .center {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .panes {
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .divider {
    width: 1px;
    background: var(--border);
  }
  .status {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    height: 26px;
    padding: 0 6px 0 12px;
    border-top: 1px solid var(--border);
    background: var(--bg);
    font-size: 11px;
    color: var(--muted);
  }
  .clip {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--accent);
  }
  .x {
    display: grid;
    padding: 2px;
    border-radius: 4px;
  }
  .x:hover {
    background: var(--hover);
  }
  .tip {
    color: var(--faint);
  }
  .transfers {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 20px;
    padding: 0 8px;
    border-radius: 5px;
  }
  .transfers:hover {
    background: var(--hover);
    color: var(--text);
  }
  .transfers.busy {
    color: var(--accent);
  }
  .mini {
    width: 60px;
    height: 4px;
    border-radius: 2px;
    background: var(--border);
    overflow: hidden;
  }
  .mini span {
    display: block;
    height: 100%;
    background: var(--accent);
  }
  .toast {
    position: fixed;
    right: 16px;
    bottom: 38px;
    z-index: 70;
    display: flex;
    align-items: center;
    gap: 8px;
    max-width: min(520px, calc(100vw - 32px));
    padding: 8px 8px 8px 12px;
    border-radius: 10px;
    background: var(--raised);
    border: 1px solid var(--border);
    box-shadow: var(--shadow);
    color: var(--ok);
  }
  .toast span {
    color: var(--text);
  }
  .toast.error {
    color: var(--danger);
    border-color: color-mix(in srgb, var(--danger) 40%, var(--border));
  }
</style>
