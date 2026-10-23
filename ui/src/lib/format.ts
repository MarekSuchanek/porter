import type { Item } from './types';

const UNITS = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'];

/** Decimal units like Finder and Explorer: `4.2 MB`. */
export function bytes(n: number | null | undefined): string {
  if (n === null || n === undefined) return '—';
  let value = n;
  let unit = 0;
  while (value >= 1000 && unit < UNITS.length - 1) {
    value /= 1000;
    unit++;
  }
  if (unit === 0) return `${n} B`;
  return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${UNITS[unit]}`;
}

const time = new Intl.DateTimeFormat(undefined, { hour: '2-digit', minute: '2-digit' });
const day = new Intl.DateTimeFormat(undefined, { day: 'numeric', month: 'short', year: 'numeric' });
const full = new Intl.DateTimeFormat(undefined, { dateStyle: 'full', timeStyle: 'medium' });

/** `Today 14:05`, `Yesterday 09:12` or `3 Oct 2026`. */
export function date(iso: string | null): string {
  if (!iso) return '—';
  const d = new Date(iso);
  const today = new Date();
  const startOfToday = new Date(today.getFullYear(), today.getMonth(), today.getDate()).getTime();
  if (d.getTime() >= startOfToday) return `Today ${time.format(d)}`;
  if (d.getTime() >= startOfToday - 86_400_000) return `Yesterday ${time.format(d)}`;
  return day.format(d);
}

export function fullDate(iso: string | null): string {
  return iso ? full.format(new Date(iso)) : '—';
}

export function duration(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return '—';
  if (seconds < 60) return `${Math.max(1, Math.round(seconds))} s`;
  const m = Math.floor(seconds / 60);
  if (m < 60) return `${m} min ${Math.round(seconds % 60)} s`;
  return `${Math.floor(m / 60)} h ${m % 60} min`;
}

const KINDS: Record<string, string> = {
  pdf: 'PDF document', doc: 'Word document', docx: 'Word document', xls: 'Spreadsheet',
  xlsx: 'Spreadsheet', csv: 'CSV table', ppt: 'Presentation', pptx: 'Presentation',
  txt: 'Plain text', md: 'Markdown', json: 'JSON', toml: 'TOML', yaml: 'YAML', yml: 'YAML',
  html: 'HTML', css: 'Stylesheet', js: 'JavaScript', ts: 'TypeScript', rs: 'Rust source',
  py: 'Python script', sh: 'Shell script', png: 'PNG image', jpg: 'JPEG image', jpeg: 'JPEG image',
  gif: 'GIF image', heic: 'HEIC image', svg: 'SVG image', webp: 'WebP image', mp3: 'Audio',
  wav: 'Audio', flac: 'Audio', mp4: 'Video', mov: 'Video', mkv: 'Video', zip: 'ZIP archive',
  gz: 'Archive', tgz: 'Archive', zst: 'Archive', tar: 'Archive', '7z': 'Archive', dmg: 'Disk image',
  iso: 'Disk image', exe: 'Application', app: 'Application',
};

export function extension(name: string): string {
  const dot = name.lastIndexOf('.');
  return dot > 0 ? name.slice(dot + 1).toLowerCase() : '';
}

export function kindLabel(item: Pick<Item, 'kind' | 'name'>): string {
  if (item.kind === 'dir') return 'Folder';
  if (item.kind === 'symlink') return 'Alias';
  if (item.kind === 'other') return 'Special file';
  const ext = extension(item.name);
  return KINDS[ext] ?? (ext ? `${ext.toUpperCase()} file` : 'File');
}

/** `rwxr-xr-x` from POSIX permission bits. */
export function modeString(mode: number): string {
  const chars = 'rwxrwxrwx';
  return [...chars].map((c, i) => (mode & (1 << (8 - i)) ? c : '-')).join('');
}

export function octal(mode: number): string {
  return (mode & 0o7777).toString(8).padStart(3, '0');
}

export function plural(n: number, one: string, many = `${one}s`): string {
  return `${n.toLocaleString()} ${n === 1 ? one : many}`;
}

const TONES: [string, string, string[]][] = [
  ['image', 'image', ['png', 'jpg', 'jpeg', 'gif', 'heic', 'svg', 'webp', 'bmp', 'tif', 'tiff', 'raw']],
  ['archive', 'archive', ['zip', 'gz', 'tgz', 'zst', 'tar', '7z', 'rar', 'xz', 'bz2', 'dmg', 'iso']],
  ['code', 'code', ['rs', 'ts', 'js', 'py', 'sh', 'json', 'toml', 'yaml', 'yml', 'html', 'css', 'svelte', 'go', 'c', 'h', 'java']],
  ['media', 'media', ['mp3', 'wav', 'flac', 'mp4', 'mov', 'mkv', 'avi', 'm4a']],
  ['text', 'file', ['txt', 'md', 'pdf', 'doc', 'docx', 'xls', 'xlsx', 'csv', 'ppt', 'pptx', 'rtf']],
];

/** Icon name and colour tone for an entry. */
export function iconFor(item: Pick<Item, 'kind' | 'name'>): { icon: string; tone: string } {
  if (item.kind === 'dir') return { icon: 'folder', tone: 'folder' };
  if (item.kind === 'symlink') return { icon: 'link', tone: 'file' };
  const ext = extension(item.name);
  for (const [icon, tone, exts] of TONES) if (exts.includes(ext)) return { icon, tone };
  return { icon: 'file', tone: 'file' };
}
