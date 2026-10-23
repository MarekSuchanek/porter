<script lang="ts">
  import { tick } from 'svelte';
  import Icon from './Icon.svelte';
  import { app } from '../lib/state.svelte';
  import { commands, shortcut } from '../lib/commands';

  interface Entry {
    title: string;
    hint: string;
    icon: string;
    run: () => void;
  }

  let query = $state('');
  let index = $state(0);
  let input = $state<HTMLInputElement>();

  $effect(() => {
    if (!app.palette) return;
    query = '';
    index = 0;
    void tick().then(() => input?.focus());
  });

  const entries = $derived.by((): Entry[] => {
    const q = query.trim().toLowerCase();
    const list: Entry[] = [];
    // Anything that looks like a location can be opened directly.
    if (q && /^(\/|~|[a-z]:[\\/]|[a-z][\w-]+:|\w+:\/\/)/i.test(query.trim())) {
      list.push({ title: `Go to ${query.trim()}`, hint: '', icon: 'forward', run: () => app.go(query.trim()) });
    }
    for (const place of [...(app.sidebar?.places ?? []), ...(app.sidebar?.remotes ?? [])]) {
      const title = `Go to ${place.name}`;
      if (!q || title.toLowerCase().includes(q)) {
        list.push({ title, hint: place.location, icon: 'icon' in place ? place.icon : 'server', run: () => app.go(place.location) });
      }
    }
    for (const c of commands) {
      if (c.id === 'palette' || (c.when && !c.when())) continue;
      if (list.some((e) => e.title === c.title)) continue; // e.g. "Go to Home" is also a place
      if (q && !`${c.group} ${c.title}`.toLowerCase().includes(q)) continue;
      list.push({ title: c.title, hint: shortcut(c.keys?.[0]), icon: 'command', run: c.run });
    }
    // Commands first when there's no query; places are a long tail otherwise.
    return q ? list : list.sort((a, b) => Number(a.title.startsWith('Go to')) - Number(b.title.startsWith('Go to')));
  });

  $effect(() => {
    void entries;
    index = 0;
  });

  function run(entry: Entry | undefined) {
    if (!entry) return;
    app.palette = false;
    entry.run();
  }

  function onKey(e: KeyboardEvent) {
    e.stopPropagation();
    if (e.key === 'Escape') app.palette = false;
    else if (e.key === 'ArrowDown') {
      e.preventDefault();
      index = Math.min(entries.length - 1, index + 1);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      index = Math.max(0, index - 1);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      run(entries[index]);
    }
  }
</script>

{#if app.palette}
  <div class="backdrop" role="presentation" onmousedown={(e) => e.target === e.currentTarget && (app.palette = false)}>
    <div class="palette" role="dialog" aria-label="Command palette">
      <div class="search">
        <Icon name="search" />
        <input bind:this={input} bind:value={query} placeholder="Type a command or a location…" spellcheck="false" onkeydown={onKey} />
      </div>
      <ul role="listbox">
        {#each entries as entry, i (i)}
          <li role="option" aria-selected={i === index}>
            <button class:active={i === index} onmousemove={() => (index = i)} onclick={() => run(entry)}>
              <Icon name={entry.icon} size={14} />
              <span class="title">{entry.title}</span>
              {#if entry.hint}<span class="hint">{entry.hint}</span>{/if}
            </button>
          </li>
        {:else}
          <li class="none">No matches</li>
        {/each}
      </ul>
    </div>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 60;
    display: grid;
    place-items: start center;
    padding-top: 12vh;
    background: rgba(10, 14, 24, 0.22);
  }
  .palette {
    width: min(560px, calc(100vw - 32px));
    border-radius: 12px;
    background: var(--raised);
    border: 1px solid var(--border);
    box-shadow: var(--shadow);
    overflow: hidden;
  }
  .search {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 14px;
    border-bottom: 1px solid var(--border);
    color: var(--faint);
  }
  .search input {
    flex: 1;
    border: 0;
    outline: 0;
    background: none;
    font-size: 15px;
    color: var(--text);
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 6px;
    max-height: 360px;
    overflow-y: auto;
  }
  button {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    height: 32px;
    padding: 0 10px;
    border-radius: 6px;
    text-align: left;
    color: var(--muted);
  }
  button.active {
    background: var(--accent);
    color: var(--accent-text);
  }
  .title {
    flex: 1;
    color: var(--text);
  }
  button.active .title,
  button.active .hint {
    color: inherit;
  }
  .hint {
    font-size: 11px;
    color: var(--faint);
    max-width: 45%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .none {
    padding: 12px;
    color: var(--faint);
    text-align: center;
  }
</style>
