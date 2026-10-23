import { SvelteSet } from 'svelte/reactivity';
import { api } from './api';
import { kindLabel } from './format';
import type {
  Conflict,
  Item,
  JobInfo,
  JobProgress,
  JobRequest,
  JobSummary,
  Listing,
  SearchProgress,
  SearchSummary,
  SidebarData,
} from './types';

export type SortKey = 'name' | 'size' | 'modified' | 'kind';

const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' });

export function message(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

// ---- preferences (persisted per browser profile) ----

export const prefs = $state({
  showHidden: false,
  inspector: true,
  drawer: false,
});

// ---- search ----

export interface SearchFilters {
  kind: '' | 'file' | 'dir';
  /** Size spec such as `+100M`, or empty. */
  size: string;
  /** Age spec such as `7d`, or empty. */
  newer: string;
  hidden: boolean;
}

/** A recursive search shown in a pane instead of the folder's contents. */
export class Search {
  id = $state<number | null>(null);
  results = $state.raw<Item[]>([]);
  running = $state(true);
  scanned = $state(0);
  matched = $state(0);
  unreadable = $state(0);
  limited = $state(false);
  error = $state<string | null>(null);

  constructor(
    readonly query: string,
    readonly root: string,
    readonly rootName: string,
    readonly filters: SearchFilters,
  ) {}

  progress(p: SearchProgress) {
    if (p.items.length) this.results = this.results.concat(p.items);
    this.scanned = p.scanned;
    this.matched = p.matched;
  }

  finish(s: SearchSummary) {
    this.running = false;
    this.scanned = s.scanned;
    this.matched = s.matched;
    this.unreadable = s.unreadable;
    this.limited = s.limited;
    this.error = s.error;
  }

  /** Path of an item's folder relative to the search root, e.g. `2024/raw`. */
  folderOf(item: Item): string {
    const relative = item.location.startsWith(this.root)
      ? item.location.slice(this.root.length).replace(/^[\\/]+/, '')
      : item.location;
    return relative.slice(0, Math.max(0, relative.length - item.name.length)).replace(/[\\/]+$/, '') || '.';
  }
}

const NO_FILTERS: SearchFilters = { kind: '', size: '', newer: '', hidden: false };

// ---- panes & tabs ----

export class Pane {
  location = $state('');
  listing = $state<Listing | null>(null);
  error = $state<string | null>(null);
  loading = $state(false);
  filter = $state('');
  sortKey = $state<SortKey>('name');
  sortDesc = $state(false);
  /** Item under the keyboard cursor. */
  cursor = $state<string | null>(null);
  selection = new SvelteSet<string>();
  canBack = $state(false);
  canForward = $state(false);
  /** Active recursive search; while set, the pane shows its results. */
  search = $state<Search | null>(null);

  #anchor: string | null = null;
  #back: string[] = [];
  #forward: string[] = [];
  #request = 0;

  /** Visible entries: hidden/filter applied, folders first, then by the sort key. */
  items = $derived.by(() => {
    const search = this.search;
    const listing = this.listing;
    if (!listing && !search) return [];
    const query = this.filter.trim().toLowerCase();
    // Search results are already filtered by the backend.
    const items = search
      ? [...search.results]
      : listing!.items.filter(
          (i) => (prefs.showHidden || !i.hidden) && (!query || i.name.toLowerCase().includes(query)),
        );
    const order = this.sortDesc ? -1 : 1;
    const compare: Record<SortKey, (a: Item, b: Item) => number> = {
      name: (a, b) => collator.compare(a.name, b.name),
      size: (a, b) => (a.size ?? -1) - (b.size ?? -1),
      modified: (a, b) => (Date.parse(a.modified ?? '') || 0) - (Date.parse(b.modified ?? '') || 0),
      kind: search
        ? (a, b) => collator.compare(search.folderOf(a), search.folderOf(b))
        : (a, b) => collator.compare(kindLabel(a), kindLabel(b)),
    };
    const byKey = compare[this.sortKey];
    return items.sort((a, b) => {
      const folders = (a.kind === 'dir' ? 0 : 1) - (b.kind === 'dir' ? 0 : 1);
      return folders || order * byKey(a, b) || collator.compare(a.name, b.name);
    });
  });

  selected = $derived(this.items.filter((i) => this.selection.has(i.location)));

  title = $derived(this.listing?.crumbs.at(-1)?.name ?? (this.location || 'Porter'));

  constructor(location?: string) {
    if (location) void this.open(location, { push: false });
  }

  async open(location: string, opts: { push?: boolean; focus?: string; keep?: boolean } = {}) {
    const { push = true, focus, keep = false } = opts;
    const request = ++this.#request;
    const previous = this.listing?.location;
    const kept = keep ? new Set(this.selection) : null;
    const cursor = this.cursor;
    this.loading = true;
    try {
      const listing = await api.listDir(location);
      if (request !== this.#request) return;
      if (push && previous && previous !== listing.location) {
        this.#back.push(previous);
        this.#forward = [];
      }
      const sameDir = previous === listing.location;
      if (!sameDir || !keep) this.exitSearch();
      this.listing = listing;
      this.location = listing.location;
      this.error = null;
      if (!sameDir) this.filter = '';
      this.selection.clear();
      if (kept) {
        const present = new Set(listing.items.map((i) => i.location));
        kept.forEach((l) => present.has(l) && this.selection.add(l));
      }
      const exists = (l: string | null | undefined) => !!l && listing.items.some((i) => i.location === l);
      this.cursor = exists(focus) ? focus! : keep && exists(cursor) ? cursor : (this.items[0]?.location ?? null);
      if (focus && exists(focus)) this.selection.add(focus);
      this.#anchor = this.cursor;
    } catch (e) {
      if (request !== this.#request) return;
      this.error = message(e);
      if (!this.listing) this.location = location;
    } finally {
      if (request === this.#request) this.loading = false;
      this.canBack = this.#back.length > 0;
      this.canForward = this.#forward.length > 0;
    }
  }

  refresh() {
    if (this.search) return this.startSearch(this.search.query, this.search.filters);
    return this.open(this.location, { push: false, keep: true });
  }

  /** Searches below the current folder; results replace the listing until exited. */
  async startSearch(query = this.filter.trim(), filters: SearchFilters = this.search?.filters ?? NO_FILTERS) {
    this.stopSearch();
    const root = this.search?.root ?? this.location;
    const rootName = this.search?.rootName ?? this.title;
    const search = new Search(query, root, rootName, { ...filters });
    this.search = search;
    this.selection.clear();
    this.cursor = null;
    try {
      const id = await api.startSearch({
        location: root,
        name: query,
        kind: filters.kind || null,
        min_size: filters.size || null,
        newer: filters.newer || null,
        hidden: filters.hidden,
      });
      search.id = id;
      app.trackSearch(id, search);
    } catch (e) {
      search.running = false;
      search.error = message(e);
    }
  }

  stopSearch() {
    const search = this.search;
    if (search?.running && search.id !== null) void api.cancelJob(search.id);
  }

  exitSearch() {
    if (!this.search) return;
    this.stopSearch();
    this.search = null;
    this.filter = '';
    this.selection.clear();
  }

  back() {
    const location = this.#back.pop();
    if (location === undefined) return;
    this.#forward.push(this.location);
    void this.open(location, { push: false });
  }

  forward() {
    const location = this.#forward.pop();
    if (location === undefined) return;
    this.#back.push(this.location);
    void this.open(location, { push: false });
  }

  up() {
    const parent = this.listing?.parent;
    if (parent) void this.open(parent, { focus: this.location });
  }

  /** Click semantics: plain, Cmd/Ctrl (toggle) or Shift (range from the anchor). */
  select(location: string, mode: 'single' | 'toggle' | 'range' = 'single') {
    if (mode === 'range' && this.#anchor) {
      const ids = this.items.map((i) => i.location);
      const [a, b] = [ids.indexOf(this.#anchor), ids.indexOf(location)].sort((x, y) => x - y);
      this.selection.clear();
      if (a >= 0) ids.slice(a, b + 1).forEach((l) => this.selection.add(l));
    } else if (mode === 'toggle') {
      if (this.selection.has(location)) this.selection.delete(location);
      else this.selection.add(location);
      this.#anchor = location;
    } else {
      this.selection.clear();
      this.selection.add(location);
      this.#anchor = location;
    }
    this.cursor = location;
  }

  selectAll() {
    this.items.forEach((i) => this.selection.add(i.location));
  }

  moveCursor(delta: number, extend = false) {
    const ids = this.items.map((i) => i.location);
    if (!ids.length) return;
    const at = this.cursor ? ids.indexOf(this.cursor) : -1;
    const next = ids[Math.max(0, Math.min(ids.length - 1, at < 0 ? 0 : at + delta))];
    this.select(next, extend ? 'range' : 'single');
  }

  setSort(key: SortKey) {
    if (this.sortKey === key) this.sortDesc = !this.sortDesc;
    else {
      this.sortKey = key;
      this.sortDesc = key === 'modified' || key === 'size';
    }
  }

  /** What actions apply to: the selection, else the item under the cursor. */
  targets(): Item[] {
    if (this.selected.length) return this.selected;
    const cursor = this.items.find((i) => i.location === this.cursor);
    return cursor ? [cursor] : [];
  }
}

let nextTabId = 1;

export class Tab {
  id = nextTabId++;
  panes = $state<Pane[]>([]);
  activeIndex = $state(0);
  dual = $state(false);

  active = $derived(this.panes[this.activeIndex] ?? this.panes[0]);
  other = $derived(this.dual ? this.panes[1 - this.activeIndex] : null);

  constructor(locations: string[], dual = false) {
    this.panes = locations.map((l) => new Pane(l));
    this.dual = dual && this.panes.length > 1;
  }

  toggleDual() {
    if (this.dual) {
      // Keep the pane the user is working in.
      if (this.activeIndex === 1) this.panes = [this.panes[1], this.panes[0]];
      this.activeIndex = 0;
      this.dual = false;
    } else {
      if (this.panes.length < 2) this.panes.push(new Pane(this.active.location));
      this.dual = true;
    }
  }
}

// ---- jobs ----

export interface JobView {
  info: JobInfo;
  progress: JobProgress | null;
  summary: JobSummary | null;
  /** Smoothed transfer rate in bytes per second. */
  rate: number;
  lastBytes: number;
  lastTime: number;
}

// ---- context menu ----

export type MenuEntry =
  | 'separator'
  | { label: string; icon?: string; keys?: string; danger?: boolean; disabled?: boolean; run: () => void };

// ---- dialogs ----

export type DialogSpec =
  | { type: 'confirm'; title: string; message: string; confirm: string; danger?: boolean }
  | { type: 'prompt'; title: string; label: string; value: string; confirm: string; selectStem?: boolean }
  | {
      type: 'choice';
      title: string;
      message: string;
      details?: string[];
      choices: { label: string; value: string; primary?: boolean; danger?: boolean }[];
    };

interface OpenDialog {
  spec: DialogSpec;
  resolve: (value: string | null) => void;
}

// ---- app ----

const STORAGE_KEY = 'porter.session.v1';

interface SavedSession {
  tabs: { locations: string[]; dual: boolean }[];
  active: number;
  prefs: typeof prefs;
}

class App {
  tabs = $state<Tab[]>([]);
  activeTabId = $state(0);
  sidebar = $state<SidebarData | null>(null);
  jobs = $state<JobView[]>([]);
  clipboard = $state<{ mode: 'copy' | 'cut'; items: Item[] } | null>(null);
  toast = $state<{ text: string; error: boolean } | null>(null);
  dialog = $state<OpenDialog | null>(null);
  palette = $state(false);
  menu = $state<{ x: number; y: number; items: MenuEntry[] } | null>(null);
  /** Bumped to ask the active path bar to switch to editing. */
  editPath = $state(0);
  /** Bumped to move focus into the active pane's filter field. */
  focusFilter = $state(0);

  tab = $derived(this.tabs.find((t) => t.id === this.activeTabId) ?? this.tabs[0]);
  pane = $derived(this.tab?.active);
  runningJobs = $derived(this.jobs.filter((j) => !j.summary).length);

  #toastTimer: ReturnType<typeof setTimeout> | undefined;
  #searches = new Map<number, Search>();
  /** Search events that arrived before the search's id was known. */
  #early = new Map<number, (SearchProgress | SearchSummary)[]>();
  #requests = new Map<number, JobRequest>();

  async init() {
    const saved = load();
    if (saved) Object.assign(prefs, saved.prefs);
    void api.on('job-started', (info) => this.#jobStarted(info));
    void api.on('job-progress', (p) => this.#jobProgress(p));
    void api.on('job-finished', (s) => this.#jobFinished(s));
    void api.on('search-progress', (p) => this.#searchEvent(p));
    void api.on('search-finished', (s) => this.#searchEvent(s));

    try {
      this.sidebar = await api.sidebar();
    } catch (e) {
      this.notify(message(e), true);
    }
    const home = this.sidebar?.places[0]?.location ?? '/';
    if (saved?.tabs.length) {
      this.tabs = saved.tabs.map((t) => new Tab(t.locations.length ? t.locations : [home], t.dual));
      this.activeTabId = this.tabs[Math.min(saved.active, this.tabs.length - 1)].id;
    } else {
      this.newTab(home);
    }
  }

  session(): SavedSession {
    return {
      tabs: this.tabs.map((t) => ({ locations: t.panes.map((p) => p.location).filter(Boolean), dual: t.dual })),
      active: Math.max(0, this.tabs.findIndex((t) => t.id === this.activeTabId)),
      prefs: { ...prefs },
    };
  }

  newTab(location = this.pane?.location ?? this.sidebar?.places[0]?.location ?? '/') {
    const tab = new Tab([location]);
    this.tabs.push(tab);
    this.activeTabId = tab.id;
  }

  closeTab(id = this.activeTabId) {
    if (this.tabs.length === 1) return;
    const index = this.tabs.findIndex((t) => t.id === id);
    this.tabs.splice(index, 1);
    if (id === this.activeTabId) this.activeTabId = this.tabs[Math.max(0, index - 1)].id;
  }

  /** Opens in the active pane. */
  go(location: string) {
    void this.pane?.open(location);
  }

  async refreshSidebar() {
    try {
      this.sidebar = await api.sidebar();
    } catch {
      // Keep what we have.
    }
  }

  notify(text: string, error = false) {
    this.toast = { text, error };
    clearTimeout(this.#toastTimer);
    this.#toastTimer = setTimeout(() => (this.toast = null), error ? 7000 : 3500);
  }

  ask(spec: DialogSpec): Promise<string | null> {
    this.dialog?.resolve(null);
    return new Promise((resolve) => {
      this.dialog = {
        spec,
        resolve: (value) => {
          this.dialog = null;
          resolve(value);
        },
      };
    });
  }

  // ---- actions ----

  async openItem(item: Item, pane = this.pane) {
    if (item.kind === 'dir') return pane.open(item.location);
    if (item.kind === 'symlink') {
      // Links to folders are browsed; links to files are opened.
      try {
        await api.listDir(item.location);
        return pane.open(item.location);
      } catch {
        /* not a folder */
      }
    }
    try {
      await api.open(item.location);
    } catch (e) {
      this.notify(message(e), true);
    }
  }

  /** Copies or moves items into `dest`, asking first if names collide. */
  async transfer(kind: 'copy' | 'move', items: Item[], dest: string) {
    if (!items.length) return;
    if (kind === 'move' && items.every((i) => parentOf(i.location) === dest)) return;
    let conflict: Conflict = 'fail';
    try {
      const existing = new Set((await api.listDir(dest)).items.map((i) => i.name));
      const clashes = items.filter((i) => existing.has(i.name)).map((i) => i.name);
      if (clashes.length) {
        const answer = await this.ask({
          type: 'choice',
          title: clashes.length === 1 ? `“${clashes[0]}” already exists` : `${clashes.length} items already exist`,
          message: `The destination already contains ${clashes.length === 1 ? 'an item' : 'items'} with the same name.`,
          details: clashes.length > 1 ? clashes.slice(0, 8) : undefined,
          choices: [
            { label: 'Cancel', value: 'cancel' },
            { label: 'Skip', value: 'skip' },
            { label: 'Replace', value: 'overwrite', danger: true },
            { label: 'Keep both', value: 'rename', primary: true },
          ],
        });
        if (!answer || answer === 'cancel') return;
        conflict = answer as Conflict;
      }
    } catch (e) {
      this.notify(message(e), true);
      return;
    }
    await this.start({ kind, sources: items.map((i) => i.location), dest, conflict });
  }

  async remove(items: Item[]) {
    if (!items.length) return;
    const what = items.length === 1 ? `“${items[0].name}”` : `${items.length} items`;
    const ok = await this.ask({
      type: 'confirm',
      title: `Delete ${what}?`,
      message: 'This permanently deletes them, including folder contents. It cannot be undone.',
      confirm: 'Delete',
      danger: true,
    });
    if (ok) await this.start({ kind: 'delete', targets: items.map((i) => i.location) });
  }

  async rename(item: Item, pane = this.pane) {
    const name = await this.ask({
      type: 'prompt',
      title: 'Rename',
      label: 'New name',
      value: item.name,
      confirm: 'Rename',
      selectStem: item.kind !== 'dir',
    });
    if (!name || name === item.name) return;
    try {
      const location = await api.rename(item.location, name);
      await pane.open(pane.location, { push: false, focus: location });
    } catch (e) {
      this.notify(message(e), true);
    }
  }

  async newFolder(pane = this.pane) {
    const name = await this.ask({
      type: 'prompt',
      title: 'New folder',
      label: 'Name',
      value: 'New folder',
      confirm: 'Create',
    });
    if (!name) return;
    try {
      const location = await api.mkdir(pane.location, name);
      await pane.open(pane.location, { push: false, focus: location });
    } catch (e) {
      this.notify(message(e), true);
    }
  }

  async chmod(items: Item[], mode: string, recursive: boolean) {
    await this.start({ kind: 'chmod', targets: items.map((i) => i.location), mode, recursive });
  }

  copyToClipboard(mode: 'copy' | 'cut', items: Item[]) {
    if (!items.length) return;
    this.clipboard = { mode, items: [...items] };
    this.notify(`${items.length === 1 ? `“${items[0].name}”` : `${items.length} items`} ready to paste`);
  }

  async paste(pane = this.pane) {
    const clip = this.clipboard;
    if (!clip) return;
    if (clip.mode === 'cut') this.clipboard = null;
    await this.transfer(clip.mode === 'cut' ? 'move' : 'copy', clip.items, pane.location);
  }

  async start(request: JobRequest) {
    try {
      const info = await api.startJob(request);
      this.#requests.set(info.id, request);
      prefs.drawer = true;
    } catch (e) {
      this.notify(message(e), true);
    }
  }

  cancelJob(id: number) {
    void api.cancelJob(id);
  }

  clearFinishedJobs() {
    this.jobs = this.jobs.filter((j) => !j.summary);
  }

  // ---- search events ----

  trackSearch(id: number, search: Search) {
    this.#searches.set(id, search);
    for (const event of this.#early.get(id) ?? []) this.#searchEvent(event);
    this.#early.delete(id);
  }

  #searchEvent(event: SearchProgress | SearchSummary) {
    const search = this.#searches.get(event.id);
    if (!search) {
      this.#early.set(event.id, [...(this.#early.get(event.id) ?? []), event]);
      return;
    }
    if ('items' in event) search.progress(event);
    else {
      search.finish(event);
      this.#searches.delete(event.id);
    }
  }

  // ---- job events ----

  #jobStarted(info: JobInfo) {
    if (this.jobs.some((j) => j.info.id === info.id)) return;
    this.jobs.unshift({ info, progress: null, summary: null, rate: 0, lastBytes: 0, lastTime: performance.now() });
  }

  #jobProgress(progress: JobProgress) {
    const job = this.jobs.find((j) => j.info.id === progress.id);
    if (!job) return;
    const now = performance.now();
    const elapsed = (now - job.lastTime) / 1000;
    if (elapsed > 0.05) {
      const instant = (progress.snapshot.bytes_done - job.lastBytes) / elapsed;
      job.rate = job.rate ? job.rate * 0.7 + instant * 0.3 : instant;
      job.lastBytes = progress.snapshot.bytes_done;
      job.lastTime = now;
    }
    job.progress = progress;
  }

  #jobFinished(summary: JobSummary) {
    const job = this.jobs.find((j) => j.info.id === summary.id);
    if (job) job.summary = summary;
    if (summary.errors.length) {
      this.notify(`${job?.info.title ?? 'Job'}: ${summary.errors.length} failed — see Transfers`, true);
    }
    // Contents and free space changed somewhere; refresh what's on screen. Search
    // results aren't re-run, but entries that were deleted or moved away disappear.
    const request = this.#requests.get(summary.id);
    this.#requests.delete(summary.id);
    const gone = new Set(
      request?.kind === 'delete' ? request.targets : request?.kind === 'move' ? request.sources : [],
    );
    for (const tab of this.tabs) {
      for (const pane of tab.dual ? tab.panes : [tab.active]) {
        if (!pane.search) void pane.refresh();
        else if (gone.size) pane.search.results = pane.search.results.filter((i) => !gone.has(i.location));
      }
    }
    void this.refreshSidebar();
  }
}

export const app = new App();

/** Parent of a location string as the backend formats them. */
export function parentOf(location: string): string {
  const trimmed = location.replace(/[\\/]+$/, '');
  const cut = Math.max(trimmed.lastIndexOf('/'), trimmed.lastIndexOf('\\'));
  if (cut < 0) return location;
  const parent = trimmed.slice(0, cut);
  return parent.endsWith(':') || parent === '' ? `${parent}/` : parent;
}

function load(): SavedSession | null {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    return raw ? (JSON.parse(raw) as SavedSession) : null;
  } catch {
    return null;
  }
}

export function save(session: SavedSession) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(session));
  } catch {
    // Storage may be unavailable (private mode); the session just isn't restored.
  }
}
