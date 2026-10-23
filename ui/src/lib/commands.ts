// Every action with a shortcut, shared by the keyboard handler and the ⌘K palette.

import { app, prefs } from './state.svelte';

export const isMac = /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);

interface Key {
  /** `KeyboardEvent.key` (case-insensitive) … */
  key?: string;
  /** … or `KeyboardEvent.code`, for keys whose `key` depends on Shift/layout. */
  code?: string;
  mod?: boolean;
  shift?: boolean;
  alt?: boolean;
}

export interface Command {
  id: string;
  title: string;
  group: string;
  keys?: Key[];
  /** Only offered/handled when this returns true. */
  when?: () => boolean;
  run: () => void;
}

const hasTargets = () => !!app.pane && app.pane.targets().length > 0;
const isDual = () => !!app.tab?.dual;
const canWrite = () => !!app.pane?.listing?.capabilities.write;

export const commands: Command[] = [
  { id: 'palette', title: 'Show all commands', group: 'General', keys: [{ key: 'k', mod: true }, { key: 'p', mod: true, shift: true }], run: () => (app.palette = !app.palette) },
  { id: 'new-tab', title: 'New tab', group: 'Tabs', keys: [{ key: 't', mod: true }], run: () => app.newTab() },
  { id: 'close-tab', title: 'Close tab', group: 'Tabs', keys: [{ key: 'w', mod: true }], when: () => app.tabs.length > 1, run: () => app.closeTab() },
  { id: 'toggle-dual', title: 'Toggle dual pane', group: 'View', keys: [{ code: 'Backslash', mod: true }], run: () => app.tab?.toggleDual() },
  { id: 'switch-pane', title: 'Switch to other pane', group: 'View', keys: [{ key: 'Tab' }], when: isDual, run: () => app.tab && (app.tab.activeIndex = 1 - app.tab.activeIndex) },
  { id: 'toggle-hidden', title: 'Show or hide hidden files', group: 'View', keys: [{ code: 'Period', mod: true, shift: true }], run: () => (prefs.showHidden = !prefs.showHidden) },
  { id: 'toggle-inspector', title: 'Show or hide inspector', group: 'View', keys: [{ key: 'i', mod: true }], run: () => (prefs.inspector = !prefs.inspector) },
  { id: 'toggle-transfers', title: 'Show or hide transfers', group: 'View', keys: [{ key: 'j', mod: true }], run: () => (prefs.drawer = !prefs.drawer) },
  { id: 'go-location', title: 'Go to location…', group: 'Go', keys: [{ key: 'l', mod: true }], run: () => app.editPath++ },
  { id: 'go-home', title: 'Go to Home', group: 'Go', keys: [{ key: 'h', mod: true, shift: true }], run: () => app.go(app.sidebar?.places[0]?.location ?? '/') },
  { id: 'back', title: 'Back', group: 'Go', keys: [{ key: '[', mod: true }, { key: 'ArrowLeft', alt: !isMac, mod: isMac }], run: () => app.pane?.back() },
  { id: 'forward', title: 'Forward', group: 'Go', keys: [{ key: ']', mod: true }, { key: 'ArrowRight', alt: !isMac, mod: isMac }], run: () => app.pane?.forward() },
  { id: 'up', title: 'Enclosing folder', group: 'Go', keys: [{ key: 'ArrowUp', mod: true }, { key: 'Backspace' }], run: () => app.pane?.up() },
  { id: 'refresh', title: 'Refresh', group: 'Go', keys: [{ key: 'r', mod: true }, { key: 'F5', mod: true }], run: () => void app.pane?.refresh() },
  { id: 'open', title: 'Open', group: 'File', keys: [{ key: 'Enter' }, { key: 'ArrowDown', mod: true }], when: hasTargets, run: () => { const [item] = app.pane.targets(); if (item) void app.openItem(item); } },
  { id: 'new-folder', title: 'New folder', group: 'File', keys: [{ key: 'n', mod: true, shift: true }], when: canWrite, run: () => void app.newFolder() },
  { id: 'rename', title: 'Rename', group: 'File', keys: [{ key: 'F2' }], when: () => app.pane?.targets().length === 1, run: () => void app.rename(app.pane.targets()[0]) },
  { id: 'delete', title: 'Delete', group: 'File', keys: [{ key: 'Delete' }, { key: 'Backspace', mod: true }], when: hasTargets, run: () => void app.remove(app.pane.targets()) },
  { id: 'copy', title: 'Copy', group: 'Edit', keys: [{ key: 'c', mod: true }], when: hasTargets, run: () => app.copyToClipboard('copy', app.pane.targets()) },
  { id: 'cut', title: 'Cut', group: 'Edit', keys: [{ key: 'x', mod: true }], when: hasTargets, run: () => app.copyToClipboard('cut', app.pane.targets()) },
  { id: 'paste', title: 'Paste', group: 'Edit', keys: [{ key: 'v', mod: true }], when: () => !!app.clipboard && canWrite(), run: () => void app.paste() },
  { id: 'select-all', title: 'Select all', group: 'Edit', keys: [{ key: 'a', mod: true }], run: () => app.pane?.selectAll() },
  { id: 'copy-other', title: 'Copy to other pane', group: 'File', keys: [{ key: 'F5' }], when: () => isDual() && hasTargets(), run: () => void app.transfer('copy', app.pane.targets(), app.tab.other!.location) },
  { id: 'move-other', title: 'Move to other pane', group: 'File', keys: [{ key: 'F6' }], when: () => isDual() && hasTargets(), run: () => void app.transfer('move', app.pane.targets(), app.tab.other!.location) },
  { id: 'clear-transfers', title: 'Clear finished transfers', group: 'View', when: () => app.jobs.some((j) => j.summary), run: () => app.clearFinishedJobs() },
];

export function matches(e: KeyboardEvent, k: Key): boolean {
  const mod = isMac ? e.metaKey : e.ctrlKey;
  const keyOk = k.code ? e.code === k.code : e.key.toLowerCase() === k.key!.toLowerCase();
  return keyOk && !!k.mod === mod && !!k.shift === e.shiftKey && !!k.alt === e.altKey;
}

const NAMES: Record<string, string> = {
  ArrowUp: '↑', ArrowDown: '↓', ArrowLeft: '←', ArrowRight: '→', Backspace: '⌫',
  Delete: 'Del', Enter: '↩', Tab: '⇥', Backslash: '\\', Period: '.',
};

/** Human-readable shortcut, e.g. `⌘⇧N` on macOS or `Ctrl+Shift+N` elsewhere. */
export function shortcut(k: Key | undefined): string {
  if (!k) return '';
  const raw = k.code ?? k.key!;
  const name = NAMES[raw] ?? (raw.length === 1 ? raw.toUpperCase() : raw);
  if (isMac) return `${k.mod ? '⌘' : ''}${k.alt ? '⌥' : ''}${k.shift ? '⇧' : ''}${name}`;
  return [k.mod && 'Ctrl', k.alt && 'Alt', k.shift && 'Shift', name].filter(Boolean).join('+');
}

export function keysFor(id: string): string {
  return shortcut(commands.find((c) => c.id === id)?.keys?.[0]);
}
