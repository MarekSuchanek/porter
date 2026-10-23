<script lang="ts">
  import Icon from './Icon.svelte';
  import { app } from '../lib/state.svelte';
  import { bytes } from '../lib/format';
  import type { Disk } from '../lib/types';

  const current = $derived(app.pane?.location);

  function usage(disk: Disk) {
    const { total_bytes: total, available_bytes: free } = disk.info;
    if (!total || free === null) return null;
    return { fraction: (total - free) / total, label: `${bytes(free)} free of ${bytes(total)}` };
  }
</script>

<nav class="sidebar" aria-label="Locations">
  {#if app.sidebar}
    <section>
      <h2>Places</h2>
      {#each app.sidebar.places as place (place.location)}
        <button class="row" class:active={current === place.location} onclick={() => app.go(place.location)}>
          <Icon name={place.icon} />
          <span class="label">{place.name}</span>
        </button>
      {/each}
    </section>

    <section>
      <h2>Disks</h2>
      {#each app.sidebar.disks as disk (disk.location)}
        {@const use = usage(disk)}
        <button
          class="row disk"
          class:active={current === disk.location}
          title={use?.label ?? disk.location}
          onclick={() => app.go(disk.location)}
        >
          <Icon name={disk.info.removable ? 'usb' : 'disk'} />
          <span class="stack">
            <span class="label">{disk.name}</span>
            {#if use}
              <span class="meter" class:full={use.fraction > 0.9}>
                <span style:width="{Math.round(use.fraction * 100)}%"></span>
              </span>
            {/if}
          </span>
        </button>
      {/each}
    </section>

    <section>
      <h2>Remotes</h2>
      {#each app.sidebar.remotes as remote (remote.name)}
        <button
          class="row"
          class:active={current?.startsWith(`${remote.name}:`)}
          title={remote.address}
          onclick={() => app.go(remote.location)}
        >
          <Icon name={remote.kind === 's3' ? 'cloud' : 'server'} />
          <span class="label">{remote.name}</span>
          <span class="badge">{remote.kind}</span>
        </button>
      {:else}
        <p class="hint">Add one with<br /><code class="selectable">porter remote add</code></p>
      {/each}
      {#if app.sidebar.config_error}
        <p class="hint error" title={app.sidebar.config_error}>Couldn't load saved remotes</p>
      {/if}
    </section>
  {/if}
</nav>

<style>
  .sidebar {
    width: 210px;
    flex: none;
    overflow-y: auto;
    padding: 10px 8px;
    background: var(--sidebar);
    border-right: 1px solid var(--border);
  }
  section + section {
    margin-top: 14px;
  }
  h2 {
    margin: 0 8px 4px;
    font-size: 11px;
    font-weight: 600;
    color: var(--faint);
    letter-spacing: 0.02em;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    min-height: 28px;
    padding: 4px 8px;
    border-radius: 6px;
    text-align: left;
    color: var(--text);
  }
  .row :global(.icon) {
    color: var(--accent);
  }
  .row:hover {
    background: var(--hover);
  }
  .row.active {
    background: var(--select);
  }
  .label {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .stack {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .meter {
    height: 3px;
    border-radius: 2px;
    background: var(--border);
    overflow: hidden;
  }
  .meter span {
    display: block;
    height: 100%;
    background: var(--accent);
    opacity: 0.75;
  }
  .meter.full span {
    background: var(--danger);
  }
  .badge {
    font-size: 10px;
    color: var(--faint);
    text-transform: uppercase;
  }
  .hint {
    margin: 4px 8px;
    font-size: 12px;
    color: var(--faint);
  }
  .hint code {
    font-size: 11px;
  }
  .hint.error {
    color: var(--danger);
  }
</style>
