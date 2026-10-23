import type {
  Item,
  JobEvents,
  JobInfo,
  JobRequest,
  Listing,
  SearchRequest,
  SidebarData,
} from './types';

type Unlisten = () => void;

export interface Backend {
  invoke<T>(command: string, args?: Record<string, unknown>): Promise<T>;
  listen<T>(event: string, handler: (payload: T) => void): Promise<Unlisten>;
}

/** Inside the app we talk to Rust; in a plain browser a mock backend stands in. */
export const inTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

let backend: Promise<Backend> | undefined;

function getBackend(): Promise<Backend> {
  backend ??= inTauri
    ? Promise.all([import('@tauri-apps/api/core'), import('@tauri-apps/api/event')]).then(
        ([core, event]): Backend => ({
          invoke: core.invoke,
          listen: (name, handler) => event.listen(name, (e) => handler(e.payload as never)),
        }),
      )
    : import('./mock').then((m) => m.mockBackend);
  return backend;
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await (await getBackend()).invoke<T>(command, args);
  } catch (e) {
    // Commands reject with the Rust error message.
    throw new Error(typeof e === 'string' ? e : e instanceof Error ? e.message : String(e));
  }
}

export const api = {
  listDir: (location: string) => call<Listing>('list_dir', { location }),
  stat: (location: string) => call<Item>('stat', { location }),
  sidebar: () => call<SidebarData>('sidebar'),
  mkdir: (parent: string, name: string) => call<string>('mkdir', { parent, name }),
  rename: (location: string, name: string) => call<string>('rename', { location, name }),
  open: (location: string) => call<void>('open', { location }),
  startJob: (request: JobRequest) => call<JobInfo>('start_job', { request }),
  cancelJob: (id: number) => call<boolean>('cancel_job', { id }),
  startSearch: (request: SearchRequest) => call<number>('start_search', { request }),
  async on<K extends keyof JobEvents>(event: K, handler: (payload: JobEvents[K]) => void) {
    return (await getBackend()).listen(event, handler);
  },
  /** Select All from the macOS app menu. */
  async onMenuSelectAll(handler: () => void) {
    return (await getBackend()).listen('menu-select-all', handler);
  },
};
