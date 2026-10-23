import { keysFor } from './commands';
import { app, parentOf, prefs, type MenuEntry, type Pane, type Tab } from './state.svelte';

/** Right-click on selected entries. */
export function itemMenu(pane: Pane, tab: Tab): MenuEntry[] {
  const items = pane.targets();
  const one = items.length === 1 ? items[0] : null;
  const caps = pane.listing?.capabilities;
  const other = tab.dual ? tab.panes[tab.panes.indexOf(pane) === 0 ? 1 : 0] : null;
  const entries: MenuEntry[] = [
    { label: 'Open', icon: 'open', keys: keysFor('open'), disabled: !one, run: () => one && app.openItem(one, pane) },
  ];
  if (one?.kind === 'dir') {
    entries.push({ label: 'Open in new tab', icon: 'plus', run: () => app.newTab(one.location) });
  }
  if (one && pane.search) {
    entries.push({
      label: 'Show in enclosing folder',
      icon: 'folder',
      run: () => pane.open(parentOf(one.location), { focus: one.location }),
    });
  }
  entries.push(
    'separator',
    { label: 'Copy', icon: 'copy', keys: keysFor('copy'), run: () => app.copyToClipboard('copy', items) },
    { label: 'Cut', icon: 'scissors', keys: keysFor('cut'), disabled: !caps?.delete, run: () => app.copyToClipboard('cut', items) },
    { label: 'Paste', icon: 'clipboard', keys: keysFor('paste'), disabled: !app.clipboard || !caps?.write, run: () => app.paste(pane) },
  );
  if (other) {
    entries.push(
      'separator',
      { label: 'Copy to other pane', icon: 'transfer', keys: keysFor('copy-other'), run: () => app.transfer('copy', items, other.location) },
      { label: 'Move to other pane', icon: 'transfer', keys: keysFor('move-other'), disabled: !caps?.delete, run: () => app.transfer('move', items, other.location) },
    );
  }
  entries.push(
    'separator',
    { label: 'Rename…', icon: 'edit', keys: keysFor('rename'), disabled: !one, run: () => one && app.rename(one, pane) },
    { label: 'Get info', icon: 'info', keys: keysFor('toggle-inspector'), run: () => (prefs.inspector = true) },
    'separator',
    { label: 'Delete…', icon: 'trash', keys: keysFor('delete'), danger: true, disabled: !caps?.delete, run: () => app.remove(items) },
  );
  return entries;
}

/** Right-click on empty space in a folder. */
export function folderMenu(pane: Pane): MenuEntry[] {
  const caps = pane.listing?.capabilities;
  return [
    { label: 'New folder…', icon: 'newfolder', keys: keysFor('new-folder'), disabled: !caps?.mkdir, run: () => app.newFolder(pane) },
    { label: 'Paste', icon: 'clipboard', keys: keysFor('paste'), disabled: !app.clipboard || !caps?.write, run: () => app.paste(pane) },
    'separator',
    { label: 'Refresh', icon: 'refresh', keys: keysFor('refresh'), run: () => pane.refresh() },
    { label: prefs.showHidden ? 'Hide hidden files' : 'Show hidden files', icon: 'eye', keys: keysFor('toggle-hidden'), run: () => (prefs.showHidden = !prefs.showHidden) },
    { label: 'Open in new tab', icon: 'plus', run: () => app.newTab(pane.location) },
  ];
}
