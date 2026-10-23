<script lang="ts">
  import Icon from './Icon.svelte';
  import type { Pane, Search, SearchFilters } from '../lib/state.svelte';

  let { pane, search }: { pane: Pane; search: Search } = $props();

  const KINDS = [
    { value: '', label: 'Any kind' },
    { value: 'file', label: 'Files' },
    { value: 'dir', label: 'Folders' },
  ];
  const SIZES = [
    { value: '', label: 'Any size' },
    { value: '+1M', label: '> 1 MB' },
    { value: '+100M', label: '> 100 MB' },
    { value: '+1G', label: '> 1 GB' },
  ];
  const AGES = [
    { value: '', label: 'Any time' },
    { value: '1d', label: 'Last 24 hours' },
    { value: '7d', label: 'Last 7 days' },
    { value: '30d', label: 'Last 30 days' },
    { value: '1y', label: 'Last year' },
  ];

  function update(change: Partial<SearchFilters>) {
    void pane.startSearch(search.query, { ...search.filters, ...change });
  }
</script>

<div class="bar" role="search">
  <div class="summary">
    <Icon name="search" size={14} />
    <span class="what">
      {search.query ? `“${search.query}”` : 'Everything'} in <strong>{search.rootName}</strong>
    </span>
    <span class="count" class:running={search.running}>
      {#if search.error}
        <span class="error" title={search.error}>{search.error}</span>
      {:else}
        {search.matched.toLocaleString()}{search.limited ? '+' : ''} found
        <span class="muted">· {search.scanned.toLocaleString()} checked</span>
        {#if search.unreadable}
          <span class="muted" title="Folders that could not be read (no permission)">· {search.unreadable} unreadable</span>
        {/if}
        {#if search.limited}<span class="muted">· showing the first {search.matched.toLocaleString()}</span>{/if}
        {#if search.running}<span class="dot" aria-label="Searching"></span>{/if}
      {/if}
    </span>
  </div>

  <div class="filters">
    <select aria-label="Kind" value={search.filters.kind} onchange={(e) => update({ kind: e.currentTarget.value as SearchFilters['kind'] })}>
      {#each KINDS as o (o.value)}<option value={o.value}>{o.label}</option>{/each}
    </select>
    <select aria-label="Size" value={search.filters.size} onchange={(e) => update({ size: e.currentTarget.value })}>
      {#each SIZES as o (o.value)}<option value={o.value}>{o.label}</option>{/each}
    </select>
    <select aria-label="Modified" value={search.filters.newer} onchange={(e) => update({ newer: e.currentTarget.value })}>
      {#each AGES as o (o.value)}<option value={o.value}>{o.label}</option>{/each}
    </select>
    <label class="check">
      <input type="checkbox" checked={search.filters.hidden} onchange={(e) => update({ hidden: e.currentTarget.checked })} />
      Hidden
    </label>
    <span class="spacer"></span>
    {#if search.running}
      <button class="btn small" onclick={() => pane.stopSearch()}>Stop</button>
    {:else}
      <button class="btn small" onclick={() => pane.refresh()}>Search again</button>
    {/if}
    <button class="btn small" title="Back to the folder (Esc)" onclick={() => pane.exitSearch()}>Done</button>
  </div>
</div>

<style>
  .bar {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 7px 10px 8px;
    border-bottom: 1px solid var(--border);
    background: var(--accent-weak);
    font-size: 12px;
    container-type: inline-size;
  }
  .summary {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    color: var(--accent);
  }
  .what {
    color: var(--text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    flex: 0 1 auto;
    min-width: 60px;
  }
  .count {
    flex: 1 1 auto;
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }
  .muted {
    color: var(--muted);
  }
  .error {
    color: var(--danger);
  }
  .dot {
    display: inline-block;
    width: 7px;
    height: 7px;
    margin-left: 6px;
    border-radius: 50%;
    background: var(--accent);
    animation: pulse 1s ease-in-out infinite;
  }
  @keyframes pulse {
    50% {
      opacity: 0.25;
    }
  }
  .filters {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
  }
  select {
    height: 24px;
    padding: 0 4px;
    border-radius: 5px;
    border: 1px solid var(--border-strong);
    background: var(--panel);
    font-size: 12px;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 4px;
    color: var(--muted);
  }
  .spacer {
    flex: 1;
  }
  .btn.small {
    height: 24px;
    padding: 0 10px;
    font-size: 12px;
  }
</style>
