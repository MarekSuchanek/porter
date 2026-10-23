<script lang="ts">
  import Icon from './Icon.svelte';
  import { app } from '../lib/state.svelte';
  import { bytes, fullDate, iconFor, kindLabel, modeString, octal, plural } from '../lib/format';
  import type { Item } from '../lib/types';

  const pane = $derived(app.pane);
  const targets = $derived(pane ? (pane.selected.length ? pane.selected : []) : []);
  const single = $derived<Item | null>(targets.length === 1 ? targets[0] : null);
  const caps = $derived(pane?.listing?.capabilities);

  // Permissions editor: a working copy of the mode bits.
  let mode = $state(0);
  let recursive = $state(false);
  const original = $derived(single?.permissions?.mode ?? null);
  $effect(() => {
    mode = original ?? 0;
    recursive = false;
  });

  const classes = [
    { label: 'Owner', shift: 6 },
    { label: 'Group', shift: 3 },
    { label: 'Others', shift: 0 },
  ];
  const bits = [
    { label: 'Read', bit: 4 },
    { label: 'Write', bit: 2 },
    { label: 'Execute', bit: 1 },
  ];

  function toggle(bit: number) {
    mode ^= bit;
  }

  function apply() {
    if (single) void app.chmod([single], octal(mode), recursive);
  }

  const totalBytes = $derived(targets.reduce((sum, i) => sum + (i.size ?? 0), 0));
  const folders = $derived(targets.filter((i) => i.kind === 'dir').length);

  function usage() {
    const fs = pane?.listing?.fs;
    if (!fs?.total_bytes || fs.available_bytes === null) return null;
    return { used: fs.total_bytes - fs.available_bytes, total: fs.total_bytes, free: fs.available_bytes };
  }
</script>

<aside class="inspector" aria-label="Inspector">
  {#if single}
    {@const { icon, tone } = iconFor(single)}
    <div class="hero">
      <span class="big {tone}"><Icon name={icon} size={44} /></span>
      <h3 class="selectable">{single.name}</h3>
      <p>{kindLabel(single)}{single.size !== null && single.kind !== 'dir' ? ` · ${bytes(single.size)}` : ''}</p>
    </div>

    <dl>
      {#if single.size !== null && single.kind !== 'dir'}
        <dt>Size</dt><dd>{single.size.toLocaleString()} bytes</dd>
      {/if}
      <dt>Modified</dt><dd>{fullDate(single.modified)}</dd>
      {#if single.created}<dt>Created</dt><dd>{fullDate(single.created)}</dd>{/if}
      {#if single.symlink_target}<dt>Points to</dt><dd class="selectable mono">{single.symlink_target}</dd>{/if}
      <dt>Where</dt><dd class="selectable mono">{single.location}</dd>
      {#if single.permissions?.uid !== null && single.permissions?.uid !== undefined}
        <dt>Owner</dt><dd>uid {single.permissions.uid} · gid {single.permissions.gid}</dd>
      {/if}
    </dl>

    <h4>Permissions</h4>
    {#if original === null}
      <p class="note">
        {caps?.set_permissions ? 'No POSIX permissions available.' : 'This storage has no file permissions.'}
      </p>
    {:else}
      <table class="perms">
        <thead>
          <tr><th></th>{#each bits as b (b.bit)}<th>{b.label}</th>{/each}</tr>
        </thead>
        <tbody>
          {#each classes as c (c.shift)}
            <tr>
              <th>{c.label}</th>
              {#each bits as b (b.bit)}
                {@const bit = b.bit << c.shift}
                <td>
                  <input
                    type="checkbox"
                    aria-label="{c.label} {b.label}"
                    checked={(mode & bit) !== 0}
                    disabled={!caps?.set_permissions}
                    onchange={() => toggle(bit)}
                  />
                </td>
              {/each}
            </tr>
          {/each}
        </tbody>
      </table>
      <p class="mode mono">{octal(mode)} · {modeString(mode)}</p>
      {#if mode !== original}
        <div class="apply">
          {#if single.kind === 'dir'}
            <label><input type="checkbox" bind:checked={recursive} /> Apply to contents</label>
          {/if}
          <button class="btn" onclick={() => (mode = original)}>Revert</button>
          <button class="btn primary" onclick={apply}>Apply</button>
        </div>
      {/if}
    {/if}
  {:else if targets.length > 1}
    <div class="hero">
      <span class="big folder"><Icon name="copy" size={44} /></span>
      <h3>{plural(targets.length, 'item')}</h3>
      <p>
        {folders ? plural(folders, 'folder') : ''}{folders && targets.length - folders ? ', ' : ''}{targets.length - folders
          ? plural(targets.length - folders, 'file')
          : ''}
      </p>
    </div>
    <dl>
      <dt>Files total</dt><dd>{bytes(totalBytes)}</dd>
    </dl>
    <p class="note">Folder contents aren't counted.</p>
  {:else if pane?.listing}
    {@const use = usage()}
    <div class="hero">
      <span class="big folder"><Icon name="folder" size={44} /></span>
      <h3 class="selectable">{pane.title}</h3>
      <p>{plural(pane.listing.items.length, 'item')}</p>
    </div>
    <dl>
      <dt>Where</dt><dd class="selectable mono">{pane.listing.location}</dd>
      {#if pane.listing.fs?.fs_type}<dt>Storage</dt><dd>{pane.listing.fs.name ?? ''} ({pane.listing.fs.fs_type})</dd>{/if}
    </dl>
    {#if use}
      <div class="usage">
        <div class="meter"><span style:width="{(use.used / use.total) * 100}%"></span></div>
        <p>{bytes(use.free)} free of {bytes(use.total)}</p>
      </div>
    {/if}
    <h4>Can do here</h4>
    <ul class="caps">
      {#each [['write', 'Write'], ['delete', 'Delete'], ['rename', 'Rename'], ['set_permissions', 'Permissions'], ['server_side_copy', 'Server-side copy']] as [key, label] (key)}
        {@const ok = caps?.[key as keyof typeof caps]}
        <li class:off={!ok}><Icon name={ok ? 'check' : 'close'} size={12} />{label}</li>
      {/each}
    </ul>
  {/if}
</aside>

<style>
  .inspector {
    width: 270px;
    flex: none;
    overflow-y: auto;
    padding: 18px 16px;
    background: var(--panel);
    border-left: 1px solid var(--border);
  }
  .hero {
    text-align: center;
    margin-bottom: 14px;
  }
  .big {
    display: inline-grid;
    color: var(--faint);
  }
  .big.folder {
    color: var(--folder);
  }
  .big.image {
    color: var(--image);
  }
  .big.archive {
    color: var(--archive);
  }
  .big.code {
    color: var(--code);
  }
  .big.media {
    color: var(--media);
  }
  h3 {
    margin: 8px 0 2px;
    font-size: 14px;
    word-break: break-word;
  }
  .hero p {
    margin: 0;
    color: var(--muted);
    font-size: 12px;
  }
  h4 {
    margin: 18px 0 8px;
    font-size: 11px;
    color: var(--faint);
    font-weight: 600;
  }
  dl {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 6px 10px;
    margin: 0;
    font-size: 12px;
  }
  dt {
    color: var(--muted);
  }
  dd {
    margin: 0;
    word-break: break-all;
  }
  .mono {
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: 11px;
  }
  .note {
    color: var(--faint);
    font-size: 12px;
  }
  .perms {
    width: 100%;
    border-collapse: collapse;
    font-size: 12px;
  }
  .perms th {
    font-weight: 500;
    color: var(--muted);
    text-align: left;
    padding: 3px 0;
  }
  .perms thead th {
    text-align: center;
    font-size: 11px;
  }
  .perms td {
    text-align: center;
  }
  .mode {
    color: var(--muted);
    margin: 8px 0 0;
  }
  .apply {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: flex-end;
    gap: 6px;
    margin-top: 10px;
  }
  .apply label {
    flex-basis: 100%;
    font-size: 12px;
    color: var(--muted);
  }
  .usage {
    margin-top: 14px;
  }
  .usage p {
    margin: 6px 0 0;
    font-size: 12px;
    color: var(--muted);
  }
  .meter {
    height: 6px;
    border-radius: 3px;
    background: var(--border);
    overflow: hidden;
  }
  .meter span {
    display: block;
    height: 100%;
    background: var(--accent);
  }
  .caps {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 4px;
    font-size: 12px;
  }
  .caps li {
    display: flex;
    gap: 6px;
    align-items: center;
    color: var(--ok);
  }
  .caps li.off {
    color: var(--faint);
  }
</style>
