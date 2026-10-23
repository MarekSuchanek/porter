<script lang="ts">
  import Icon from './Icon.svelte';
  import { app, prefs, type JobView } from '../lib/state.svelte';
  import { bytes, duration, plural } from '../lib/format';

  let expanded = $state<Set<number>>(new Set());

  const ICONS = { copy: 'copy', move: 'transfer', delete: 'trash', chmod: 'lock' } as const;

  function fraction(job: JobView): number {
    const s = job.progress?.snapshot;
    if (!s) return 0;
    if (job.summary) return 1;
    if (s.bytes_total > 0) return s.bytes_done / s.bytes_total;
    return s.files_total > 0 ? s.files_done / s.files_total : 0;
  }

  function detail(job: JobView): string {
    const s = job.progress?.snapshot;
    if (!s) return 'Starting…';
    if (s.scanning) return `Scanning… ${plural(s.scanned, 'item')} found`;
    const parts: string[] = [];
    if (s.bytes_total > 0) {
      parts.push(`${bytes(s.bytes_done)} of ${bytes(s.bytes_total)}`);
      if (job.rate > 0) {
        parts.push(`${bytes(job.rate)}/s`);
        parts.push(`${duration((s.bytes_total - s.bytes_done) / job.rate)} left`);
      }
    }
    parts.push(`${s.files_done.toLocaleString()} of ${plural(s.files_total, 'item')}`);
    return parts.join(' · ');
  }

  function outcome(job: JobView): string {
    const s = job.summary!;
    if (s.cancelled) return 'Cancelled';
    const done = [s.files && plural(s.files, 'file'), s.dirs && plural(s.dirs, 'folder')].filter(Boolean).join(', ');
    const parts = [done || 'Nothing to do'];
    if (s.bytes) parts.push(bytes(s.bytes));
    if (s.skipped) parts.push(`${s.skipped} skipped`);
    if (s.errors.length) parts.push(`${s.errors.length} failed`);
    return parts.join(' · ');
  }

  function toggle(id: number) {
    const next = new Set(expanded);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    expanded = next;
  }
</script>

<section class="drawer" aria-label="Transfers">
  <header>
    <h2>Transfers</h2>
    <span class="count">{app.runningJobs ? `${app.runningJobs} running` : ''}</span>
    <button class="btn small" disabled={!app.jobs.some((j) => j.summary)} onclick={() => app.clearFinishedJobs()}>Clear finished</button>
    <button class="tool" aria-label="Hide transfers" onclick={() => (prefs.drawer = false)}><Icon name="chevron" /></button>
  </header>

  <ul>
    {#each app.jobs as job (job.info.id)}
      {@const s = job.summary}
      <li class:failed={s && s.errors.length} class:done={s && !s.errors.length && !s.cancelled}>
        <span class="kind"><Icon name={s?.errors.length ? 'alert' : s && !s.cancelled ? 'check' : ICONS[job.info.kind]} /></span>
        <div class="body">
          <div class="title" title={job.info.title}>{job.info.title}</div>
          {#if !s}
            <div class="bar" class:indeterminate={!job.progress || job.progress.snapshot.scanning}>
              <span style:width="{fraction(job) * 100}%"></span>
            </div>
            <div class="detail">{detail(job)}</div>
            {#if job.progress?.current}<div class="current" title={job.progress.current}>{job.progress.current}</div>{/if}
          {:else}
            <div class="detail">
              {outcome(job)}
              {#if s.errors.length}
                <button class="link" onclick={() => toggle(job.info.id)}>{expanded.has(job.info.id) ? 'Hide' : 'Show'} errors</button>
              {/if}
            </div>
            {#if expanded.has(job.info.id)}
              <ul class="errors selectable">
                {#each s.errors as error, i (i)}<li>{error}</li>{/each}
              </ul>
            {/if}
          {/if}
        </div>
        {#if !s}
          <button class="tool" title="Cancel" aria-label="Cancel {job.info.title}" onclick={() => app.cancelJob(job.info.id)}><Icon name="close" /></button>
        {/if}
      </li>
    {:else}
      <li class="empty">No transfers yet. Copy, move or delete something and it shows up here.</li>
    {/each}
  </ul>
</section>

<style>
  .drawer {
    display: flex;
    flex-direction: column;
    max-height: 260px;
    min-height: 120px;
    background: var(--panel);
    border-top: 1px solid var(--border);
  }
  header {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 10px 6px 14px;
    border-bottom: 1px solid var(--border);
  }
  h2 {
    margin: 0;
    font-size: 12px;
    font-weight: 600;
  }
  .count {
    flex: 1;
    font-size: 12px;
    color: var(--accent);
  }
  .btn.small {
    height: 22px;
    padding: 0 8px;
    font-size: 11px;
  }
  .btn:disabled {
    opacity: 0.4;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    overflow-y: auto;
  }
  li {
    display: flex;
    gap: 10px;
    align-items: flex-start;
    padding: 8px 12px 8px 14px;
    border-bottom: 1px solid var(--border);
  }
  li.empty {
    color: var(--faint);
    justify-content: center;
    padding: 18px;
  }
  .kind {
    display: grid;
    padding-top: 1px;
    color: var(--accent);
  }
  li.done .kind {
    color: var(--ok);
  }
  li.failed .kind {
    color: var(--danger);
  }
  .body {
    flex: 1;
    min-width: 0;
  }
  .title,
  .current {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .title {
    font-weight: 500;
  }
  .detail,
  .current {
    font-size: 11px;
    color: var(--muted);
    font-variant-numeric: tabular-nums;
  }
  .current {
    color: var(--faint);
  }
  .bar {
    height: 5px;
    margin: 6px 0 4px;
    border-radius: 3px;
    background: var(--border);
    overflow: hidden;
  }
  .bar span {
    display: block;
    height: 100%;
    background: var(--accent);
    transition: width 0.15s linear;
  }
  .bar.indeterminate span {
    width: 30% !important;
    animation: slide 1.1s ease-in-out infinite;
  }
  @keyframes slide {
    from {
      transform: translateX(-100%);
    }
    to {
      transform: translateX(340%);
    }
  }
  .link {
    margin-left: 6px;
    color: var(--accent);
    text-decoration: underline;
  }
  .errors {
    margin-top: 4px;
    font-size: 11px;
    color: var(--danger);
  }
  .errors li {
    padding: 2px 0;
    border: 0;
  }
</style>
