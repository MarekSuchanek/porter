<script lang="ts">
  import Icon from './Icon.svelte';
  import { app, type Pane, type SortKey, type Tab } from '../lib/state.svelte';
  import { bytes, date, iconFor, kindLabel } from '../lib/format';
  import { dragOver, drop, endDrag, startDrag } from '../lib/dnd';
  import { folderMenu, itemMenu } from '../lib/menus';
  import { isMac } from '../lib/commands';
  import type { Item } from '../lib/types';

  let { pane, tab, active }: { pane: Pane; tab: Tab; active: boolean } = $props();

  const ROW = 28;
  const OVERSCAN = 10;

  let viewport = $state<HTMLDivElement>();
  let scrollTop = $state(0);
  let height = $state(600);
  let dropFolder = $state<string | null>(null);

  const first = $derived(Math.max(0, Math.floor(scrollTop / ROW) - OVERSCAN));
  const last = $derived(Math.min(pane.items.length, Math.ceil((scrollTop + height) / ROW) + OVERSCAN));
  const rows = $derived(pane.items.slice(first, last));
  const cut = $derived(
    app.clipboard?.mode === 'cut' ? new Set(app.clipboard.items.map((i) => i.location)) : null,
  );

  const columns: { key: SortKey; label: string; align?: 'end' }[] = [
    { key: 'name', label: 'Name' },
    { key: 'size', label: 'Size', align: 'end' },
    { key: 'modified', label: 'Modified' },
    { key: 'kind', label: 'Kind' },
  ];

  // New folder: start at the top.
  $effect(() => {
    void pane.location;
    if (viewport) viewport.scrollTop = 0;
  });

  // Keep the keyboard cursor in view.
  $effect(() => {
    const cursor = pane.cursor;
    if (!cursor || !viewport) return;
    const index = pane.items.findIndex((i) => i.location === cursor);
    if (index < 0) return;
    const top = index * ROW;
    if (top < viewport.scrollTop) viewport.scrollTop = top;
    else if (top + ROW > viewport.scrollTop + viewport.clientHeight) {
      viewport.scrollTop = top + ROW - viewport.clientHeight;
    }
  });

  function onRowClick(e: MouseEvent, item: Item) {
    const toggle = isMac ? e.metaKey : e.ctrlKey;
    pane.select(item.location, e.shiftKey ? 'range' : toggle ? 'toggle' : 'single');
  }

  function onRowMenu(e: MouseEvent, item: Item) {
    e.preventDefault();
    e.stopPropagation();
    if (!pane.selection.has(item.location)) pane.select(item.location);
    app.menu = { x: e.clientX, y: e.clientY, items: itemMenu(pane, tab) };
  }

  function onBackgroundMenu(e: MouseEvent) {
    e.preventDefault();
    pane.selection.clear();
    app.menu = { x: e.clientX, y: e.clientY, items: folderMenu(pane) };
  }

  function onDragStart(e: DragEvent, item: Item) {
    if (!pane.selection.has(item.location)) pane.select(item.location);
    startDrag(e, pane.selected.length ? pane.selected : [item]);
  }
</script>

<div class="list" role="grid" aria-label="Files" aria-rowcount={pane.items.length}>
  <div class="head" role="row">
    {#each columns as col (col.key)}
      <button
        class="cell c-{col.key}"
        class:end={col.align === 'end'}
        class:sorted={pane.sortKey === col.key}
        role="columnheader"
        aria-sort={pane.sortKey === col.key ? (pane.sortDesc ? 'descending' : 'ascending') : 'none'}
        onclick={() => pane.setSort(col.key)}
      >
        {col.key === 'kind' && pane.search ? 'Folder' : col.label}
        {#if pane.sortKey === col.key}<span class="arrow">{pane.sortDesc ? '▾' : '▴'}</span>{/if}
      </button>
    {/each}
  </div>

  <div
    class="viewport"
    bind:this={viewport}
    bind:clientHeight={height}
    onscroll={() => (scrollTop = viewport!.scrollTop)}
    onclick={(e) => e.target === e.currentTarget && pane.selection.clear()}
    oncontextmenu={onBackgroundMenu}
    onkeydown={() => {}}
    role="presentation"
  >
    <div class="spacer" style:height="{pane.items.length * ROW}px">
      {#each rows as item, i (item.location)}
        {@const { icon, tone } = iconFor(item)}
        {@const selected = pane.selection.has(item.location)}
        <div
          class="row"
          class:selected
          class:focused={active}
          class:cursor={active && pane.cursor === item.location}
          class:hidden={item.hidden}
          class:cut={cut?.has(item.location)}
          class:drop={dropFolder === item.location}
          style:transform="translateY({(first + i) * ROW}px)"
          role="row"
          tabindex="-1"
          aria-selected={selected}
          draggable="true"
          onclick={(e) => onRowClick(e, item)}
          ondblclick={() => app.openItem(item, pane)}
          oncontextmenu={(e) => onRowMenu(e, item)}
          onkeydown={() => {}}
          ondragstart={(e) => onDragStart(e, item)}
          ondragend={endDrag}
          ondragover={(e) => {
            if (item.kind === 'dir' && dragOver(e, item.location)) {
              e.stopPropagation();
              dropFolder = item.location;
            }
          }}
          ondragleave={() => dropFolder === item.location && (dropFolder = null)}
          ondrop={(e) => {
            dropFolder = null;
            if (item.kind === 'dir') drop(e, item.location);
          }}
        >
          <span class="cell name" role="gridcell">
            <span class="tone {tone}"><Icon name={icon} /></span>
            <span class="text" title={item.name}>{item.name}</span>
          </span>
          <span class="cell c-size end muted" role="gridcell">{item.kind === 'dir' ? '—' : bytes(item.size)}</span>
          <span class="cell c-modified muted" role="gridcell">{date(item.modified)}</span>
          {#if pane.search}
            {@const folder = pane.search.folderOf(item)}
            <span class="cell c-kind muted" role="gridcell" title={item.location}>{folder}</span>
          {:else}
            <span class="cell c-kind muted" role="gridcell">{kindLabel(item)}</span>
          {/if}
        </div>
      {/each}
    </div>

    {#if pane.search && !pane.items.length}
      <div class="empty">{pane.search.running ? 'Searching…' : pane.search.error ? '' : 'No matches'}</div>
    {:else if pane.listing && !pane.items.length}
      <div class="empty">
        {#if pane.filter}
          No items match “{pane.filter}”
        {:else if pane.listing.items.length}
          Only hidden items here
        {:else}
          This folder is empty
        {/if}
      </div>
    {/if}
  </div>
</div>

<style>
  .list {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    container-type: inline-size;
    --cols: minmax(160px, 1fr) 76px 136px 120px;
  }
  /* Narrow panes (dual-pane, inspector open) drop columns instead of overflowing. */
  @container (max-width: 600px) {
    .head,
    .row {
      grid-template-columns: minmax(120px, 1fr) 70px 126px;
    }
    .c-kind {
      display: none;
    }
  }
  @container (max-width: 420px) {
    .head,
    .row {
      grid-template-columns: minmax(100px, 1fr) 66px;
    }
    .c-modified {
      display: none;
    }
  }
  .head,
  .row {
    display: grid;
    grid-template-columns: var(--cols);
    align-items: center;
    column-gap: 12px;
    padding: 0 12px;
  }
  .head {
    height: 26px;
    border-bottom: 1px solid var(--border);
    font-size: 12px;
    color: var(--muted);
  }
  .head .cell {
    text-align: left;
    padding: 4px 0;
  }
  .head .cell:hover,
  .head .sorted {
    color: var(--text);
  }
  .arrow {
    font-size: 10px;
    margin-left: 2px;
  }
  .viewport {
    position: relative;
    flex: 1;
    overflow-y: auto;
    overflow-x: hidden;
    outline: none;
  }
  .spacer {
    position: relative;
  }
  .row {
    position: absolute;
    left: 4px;
    right: 4px;
    height: var(--row);
    border-radius: 6px;
    white-space: nowrap;
  }
  .row:hover {
    background: var(--hover);
  }
  .row:focus {
    outline: none;
  }
  .row.selected {
    background: var(--select);
  }
  .row.selected.focused {
    background: color-mix(in srgb, var(--select-focus) 22%, var(--panel));
  }
  .row.cursor {
    box-shadow: inset 0 0 0 1.5px var(--select-focus);
  }
  .row.hidden .text,
  .row.cut {
    opacity: 0.5;
  }
  .row.drop {
    background: var(--accent-weak);
    box-shadow: inset 0 0 0 2px var(--accent);
  }
  .cell {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .cell.end {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .muted {
    color: var(--muted);
    font-size: 12px;
  }
  .name {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .text {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .tone {
    display: grid;
    color: var(--faint);
  }
  .tone.folder {
    color: var(--folder);
  }
  .tone.image {
    color: var(--image);
  }
  .tone.archive {
    color: var(--archive);
  }
  .tone.code {
    color: var(--code);
  }
  .tone.media {
    color: var(--media);
  }
  .empty {
    position: absolute;
    inset: 40px 0 auto;
    text-align: center;
    color: var(--faint);
  }
</style>
