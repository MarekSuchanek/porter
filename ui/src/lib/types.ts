// Mirrors the Rust types serialized by porter-gui (snake_case fields).

export type EntryKind = 'file' | 'dir' | 'symlink' | 'other';

export interface Permissions {
  mode: number | null;
  readonly: boolean;
  uid: number | null;
  gid: number | null;
}

export interface Item {
  name: string;
  path: string;
  location: string;
  kind: EntryKind;
  size: number | null;
  modified: string | null;
  created: string | null;
  accessed: string | null;
  permissions: Permissions | null;
  hidden: boolean;
  symlink_target: string | null;
}

export interface FsInfo {
  name: string | null;
  fs_type: string | null;
  mount_point: string | null;
  total_bytes: number | null;
  available_bytes: number | null;
  read_only: boolean | null;
  removable: boolean | null;
}

export interface Capabilities {
  list: boolean;
  read: boolean;
  write: boolean;
  delete: boolean;
  rename: boolean;
  mkdir: boolean;
  set_permissions: boolean;
  set_owner: boolean;
  set_modified: boolean;
  symlinks: boolean;
  server_side_copy: boolean;
  fs_info: boolean;
}

export interface Crumb {
  name: string;
  location: string;
}

export interface Listing {
  location: string;
  crumbs: Crumb[];
  parent: string | null;
  items: Item[];
  capabilities: Capabilities;
  fs: FsInfo | null;
}

export interface Place {
  name: string;
  location: string;
  icon: string;
}

export interface Disk {
  name: string;
  location: string;
  info: FsInfo;
}

export interface RemoteItem {
  name: string;
  kind: string;
  address: string;
  location: string;
}

export interface SidebarData {
  places: Place[];
  disks: Disk[];
  remotes: RemoteItem[];
  config_error: string | null;
}

export type Conflict = 'fail' | 'skip' | 'overwrite' | 'rename' | 'newer';

export type JobRequest =
  | { kind: 'copy' | 'move'; sources: string[]; dest: string; conflict: Conflict }
  | { kind: 'delete'; targets: string[] }
  | { kind: 'chmod'; targets: string[]; mode: string; recursive: boolean };

export interface JobInfo {
  id: number;
  kind: 'copy' | 'move' | 'delete' | 'chmod';
  title: string;
}

export interface Snapshot {
  scanned: number;
  files_total: number;
  files_done: number;
  bytes_total: number;
  bytes_done: number;
  scanning: boolean;
}

export interface JobProgress {
  id: number;
  snapshot: Snapshot;
  current: string | null;
}

export interface JobSummary {
  id: number;
  files: number;
  dirs: number;
  bytes: number;
  skipped: number;
  errors: string[];
  cancelled: boolean;
}

export interface SearchRequest {
  location: string;
  /** Glob (`*.pdf`) or text contained in the name; empty matches everything. */
  name: string;
  kind: 'file' | 'dir' | null;
  /** Size spec such as `+100M`. */
  min_size: string | null;
  /** Age spec such as `7d`. */
  newer: string | null;
  hidden: boolean;
}

export interface SearchProgress {
  id: number;
  items: Item[];
  scanned: number;
  matched: number;
}

export interface SearchSummary {
  id: number;
  matched: number;
  scanned: number;
  unreadable: number;
  cancelled: boolean;
  limited: boolean;
  error: string | null;
}

export interface JobEvents {
  'job-started': JobInfo;
  'job-progress': JobProgress;
  'job-finished': JobSummary;
  'search-progress': SearchProgress;
  'search-finished': SearchSummary;
}
