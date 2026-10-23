// In-memory stand-in for the Rust backend, used when the UI runs in a plain browser
// (`npm run dev`) so it can be developed and checked without the app shell.

import type { Backend } from './api';
import type {
  Capabilities,
  Crumb,
  FsInfo,
  Item,
  JobInfo,
  JobRequest,
  JobSummary,
  Listing,
  SidebarData,
} from './types';

interface Node {
  name: string;
  dir: boolean;
  size: number;
  modified: string;
  mode: number | null;
  children?: Map<string, Node>;
}

const DAY = 86_400_000;
const now = Date.now();
let seed = 7;
function random() {
  seed = (seed * 16807) % 2147483647;
  return seed / 2147483647;
}

function file(name: string, size: number, daysAgo = random() * 400, mode: number | null = 0o644): Node {
  return { name, dir: false, size, modified: new Date(now - daysAgo * DAY).toISOString(), mode };
}

function dir(name: string, children: Node[] = [], mode: number | null = 0o755): Node {
  return {
    name,
    dir: true,
    size: 0,
    modified: new Date(now - random() * 200 * DAY).toISOString(),
    mode,
    children: new Map(children.map((c) => [c.name, c])),
  };
}

function remoteNode(node: Node): Node {
  node.mode = null;
  node.children?.forEach(remoteNode);
  return node;
}

const downloads = Array.from({ length: 240 }, (_, i) =>
  file(`download-${String(i + 1).padStart(3, '0')}.${['zip', 'pdf', 'dmg', 'png'][i % 4]}`, Math.round(random() * 400e6)),
);

const roots: Record<string, Node> = {
  '': dir('/', [
    dir('Users', [
      dir('demo', [
        dir('Desktop', [file('notes.md', 2_400, 1), file('screenshot.png', 1_820_000, 0.2)]),
        dir('Documents', [
          dir('Projects', [dir('porter', [file('README.md', 3_100), file('Cargo.toml', 420)])]),
          file('Budget 2026.xlsx', 48_200, 12),
          file('Contract.pdf', 1_240_000, 30),
          file('Thesis.docx', 5_300_000, 90),
        ]),
        dir('Downloads', downloads),
        dir('Pictures', [
          dir('2024 Summer', [file('beach.jpg', 4_200_000), file('sunset.jpg', 3_900_000)]),
          file('avatar.png', 220_000),
        ]),
        file('.zshrc', 1_200, 40, 0o600),
        file('todo.txt', 600, 2),
      ]),
    ]),
    dir('Volumes', [dir('Backup', [file('archive.tar.gz', 8_400_000_000)])]),
  ]),
  nas: remoteNode(
    dir('/', [
      dir('photos', [dir('2023'), dir('2024', [file('IMG_0001.HEIC', 2_900_000), file('IMG_0002.HEIC', 3_100_000)])]),
      dir('media', [file('movie.mkv', 7_300_000_000)]),
      file('readme.txt', 900),
    ]),
  ),
  backup: remoteNode(dir('/', [dir('laptop', [file('2026-10-01.tar.zst', 2_100_000_000)])])),
};

const remoteCaps: Capabilities = {
  list: true, read: true, write: true, delete: true, rename: false, mkdir: true,
  set_permissions: false, set_owner: false, set_modified: false, symlinks: false,
  server_side_copy: true, fs_info: true,
};
const localCaps: Capabilities = { ...remoteCaps, rename: true, set_permissions: true, set_modified: true, symlinks: true, server_side_copy: false };

// ---- locations ----

interface Parsed { fs: string; parts: string[] }

function parse(location: string): Parsed {
  const remote = /^([A-Za-z][\w-]+):(.*)$/.exec(location);
  const [fs, path] = remote ? [remote[1], remote[2]] : ['', location];
  if (!(fs in roots)) throw `invalid location \`${location}:\`: unknown remote`;
  return { fs, parts: path.split('/').filter(Boolean) };
}

function format(p: Parsed): string {
  const path = '/' + p.parts.join('/');
  return p.fs ? `${p.fs}:${path}` : path;
}

function lookup(p: Parsed): Node {
  let node = roots[p.fs];
  for (const part of p.parts) {
    const next = node.children?.get(part);
    if (!next) throw `${format(p)}: not found`;
    node = next;
  }
  return node;
}

function parentOf(p: Parsed): Parsed | null {
  return p.parts.length ? { fs: p.fs, parts: p.parts.slice(0, -1) } : null;
}

function toItem(node: Node, p: Parsed): Item {
  return {
    name: node.name,
    path: '/' + p.parts.join('/'),
    location: format(p),
    kind: node.dir ? 'dir' : 'file',
    size: node.dir ? null : node.size,
    modified: node.modified,
    created: null,
    accessed: null,
    permissions: node.mode === null ? null : { mode: node.mode, readonly: !(node.mode & 0o200), uid: 501, gid: 20 },
    hidden: node.name.startsWith('.'),
    symlink_target: null,
  };
}

function fsInfo(fs: string): FsInfo {
  if (fs) return { name: fs, fs_type: fs === 'nas' ? 'sftp' : 's3', mount_point: null, total_bytes: null, available_bytes: null, read_only: null, removable: null };
  return { name: 'Macintosh HD', fs_type: 'apfs', mount_point: '/', total_bytes: 994e9, available_bytes: 412e9, read_only: false, removable: false };
}

function treeSize(node: Node): { files: number; bytes: number } {
  if (!node.dir) return { files: 1, bytes: node.size };
  let files = 0, bytes = 0;
  node.children?.forEach((c) => { const s = treeSize(c); files += s.files; bytes += s.bytes; });
  return { files, bytes };
}

function clone(node: Node): Node {
  return { ...node, children: node.children && new Map([...node.children].map(([k, v]) => [k, clone(v)])) };
}

function uniqueName(dir: Node, name: string): string {
  const dot = name.lastIndexOf('.');
  const [stem, ext] = dot > 0 ? [name.slice(0, dot), name.slice(dot)] : [name, ''];
  for (let n = 1; ; n++) {
    const candidate = `${stem} (${n})${ext}`;
    if (!dir.children!.has(candidate)) return candidate;
  }
}

// ---- events & jobs ----

const listeners = new Map<string, Set<(payload: unknown) => void>>();
function emit(event: string, payload: unknown) {
  listeners.get(event)?.forEach((h) => h(payload));
}

let nextJob = 1;
const cancelled = new Set<number>();

function runJob(request: JobRequest): JobInfo {
  const locations: string[] = 'sources' in request ? request.sources : request.targets;
  const nodes = locations.map((l) => ({ p: parse(l), node: lookup(parse(l)) }));
  const dest = 'dest' in request ? parse(request.dest) : null;
  const destNode = dest ? lookup(dest) : null;
  if (dest && destNode && !destNode.dir) throw `${format(dest)}: target is not a directory`;

  const what = nodes.length === 1 ? `“${nodes[0].node.name}”` : `${nodes.length} items`;
  const verb = { copy: 'Copying', move: 'Moving', delete: 'Deleting', chmod: 'Changing permissions of' }[request.kind];
  const info: JobInfo = { id: nextJob++, kind: request.kind, title: `${verb} ${what}${dest ? ` to ${format(dest)}` : ''}` };

  const totals = nodes.reduce((t, { node }) => { const s = treeSize(node); return { files: t.files + s.files, bytes: t.bytes + s.bytes }; }, { files: 0, bytes: 0 });
  const transfers = request.kind === 'copy' || request.kind === 'move';
  const bytesTotal = transfers ? totals.bytes : 0;
  const steps = 20;
  let step = 0;
  setTimeout(() => emit('job-started', info));

  const timer = setInterval(() => {
    step++;
    const done = cancelled.has(info.id) || step >= steps;
    const fraction = Math.min(step / steps, 1);
    emit('job-progress', {
      id: info.id,
      current: nodes[Math.min(nodes.length - 1, Math.floor(fraction * nodes.length))].p.parts.join('/'),
      snapshot: {
        scanned: totals.files, scanning: false, files_total: totals.files,
        files_done: Math.round(totals.files * fraction), bytes_total: bytesTotal, bytes_done: Math.round(bytesTotal * fraction),
      },
    });
    if (!done) return;
    clearInterval(timer);

    const summary: JobSummary = { id: info.id, files: 0, dirs: 0, bytes: 0, skipped: 0, errors: [], cancelled: cancelled.has(info.id) };
    if (!summary.cancelled) {
      for (const { p, node } of nodes) {
        const parent = lookup(parentOf(p) ?? p);
        if (request.kind === 'delete') parent.children!.delete(node.name);
        if (request.kind === 'chmod' && node.mode !== null) node.mode = parseInt(request.mode, 8);
        if (request.kind === 'chmod' && node.mode === null) summary.errors.push(`\`set_permissions\` is not supported by the ${fsInfo(p.fs).fs_type} backend`);
        if (destNode && 'conflict' in request) {
          let name = node.name;
          if (destNode.children!.has(name)) {
            if (request.conflict === 'skip') { summary.skipped++; continue; }
            if (request.conflict === 'rename') name = uniqueName(destNode, name);
            if (request.conflict === 'fail') { summary.errors.push(`${format({ fs: dest!.fs, parts: [...dest!.parts, name] })}: already exists`); continue; }
          }
          destNode.children!.set(name, { ...clone(node), name });
          if (request.kind === 'move') parent.children!.delete(node.name);
        }
        const s = treeSize(node);
        summary.files += s.files;
        summary.bytes += transfers ? s.bytes : 0;
      }
    }
    cancelled.delete(info.id);
    emit('job-finished', summary);
  }, 120);
  return info;
}

// ---- commands ----

const commands: Record<string, (args: Record<string, any>) => unknown> = {
  list_dir({ location }): Listing {
    const p = parse(location);
    const node = lookup(p);
    if (!node.dir) throw `${location}: not a directory`;
    const crumbs: Crumb[] = [];
    for (let q: Parsed | null = p; q; q = parentOf(q)) {
      crumbs.unshift({ name: q.parts.at(-1) ?? (q.fs || '/'), location: format(q) });
    }
    const parent = parentOf(p);
    return {
      location: format(p),
      crumbs,
      parent: parent && format(parent),
      items: [...node.children!.values()].map((c) => toItem(c, { fs: p.fs, parts: [...p.parts, c.name] })),
      capabilities: p.fs ? remoteCaps : localCaps,
      fs: fsInfo(p.fs),
    };
  },
  stat({ location }): Item {
    const p = parse(location);
    return toItem(lookup(p), p);
  },
  sidebar(): SidebarData {
    const home = '/Users/demo';
    return {
      places: [
        { name: 'Home', location: home, icon: 'home' },
        { name: 'Desktop', location: `${home}/Desktop`, icon: 'desktop' },
        { name: 'Documents', location: `${home}/Documents`, icon: 'documents' },
        { name: 'Downloads', location: `${home}/Downloads`, icon: 'downloads' },
        { name: 'Pictures', location: `${home}/Pictures`, icon: 'pictures' },
      ],
      disks: [
        { name: 'Macintosh HD', location: '/', info: fsInfo('') },
        { name: 'Backup', location: '/Volumes/Backup', info: { ...fsInfo(''), name: 'Backup', mount_point: '/Volumes/Backup', total_bytes: 2e12, available_bytes: 0.31e12, removable: true } },
      ],
      remotes: [
        { name: 'nas', kind: 'sftp', address: 'sftp://me@nas.local', location: 'nas:/' },
        { name: 'backup', kind: 's3', address: 's3://laptop-backups', location: 'backup:/' },
      ],
      config_error: null,
    };
  },
  mkdir({ parent, name }): string {
    const p = parse(parent);
    const node = lookup(p);
    if (node.children!.has(name)) throw `${format({ ...p, parts: [...p.parts, name] })}: already exists`;
    node.children!.set(name, dir(name, [], p.fs ? null : 0o755));
    return format({ ...p, parts: [...p.parts, name] });
  },
  rename({ location, name }): string {
    const p = parse(location);
    const node = lookup(p);
    const parent = lookup(parentOf(p)!);
    if (parent.children!.has(name)) throw `${name}: already exists`;
    parent.children!.delete(node.name);
    node.name = name;
    parent.children!.set(name, node);
    return format({ ...p, parts: [...p.parts.slice(0, -1), name] });
  },
  open({ location }) {
    if (parse(location).fs) throw `${location}: only local files can be opened (copy it to this computer first)`;
    console.info('[mock] open', location);
  },
  start_job: ({ request }) => runJob(request),
  cancel_job({ id }) {
    cancelled.add(id);
    return true;
  },
};

export const mockBackend: Backend = {
  async invoke<T>(command: string, args: Record<string, unknown> = {}) {
    await new Promise((r) => setTimeout(r, 40)); // feel a little like IPC
    const handler = commands[command];
    if (!handler) throw `unknown command ${command}`;
    return handler(args) as T;
  },
  async listen<T>(event: string, handler: (payload: T) => void) {
    const set = listeners.get(event) ?? new Set();
    listeners.set(event, set);
    set.add(handler as (p: unknown) => void);
    return () => set.delete(handler as (p: unknown) => void);
  },
};
