// Drag and drop of entries between panes, tabs' path bars and folders.
// Drop copies; holding Shift moves.

import { app, parentOf } from './state.svelte';
import type { Item } from './types';

let dragged: Item[] | null = null;

export function startDrag(e: DragEvent, items: Item[]) {
  dragged = items;
  if (!e.dataTransfer) return;
  e.dataTransfer.effectAllowed = 'copyMove';
  e.dataTransfer.setData('text/plain', items.map((i) => i.location).join('\n'));
}

export function endDrag() {
  dragged = null;
}

/** Whether the dragged items may be dropped into `dest`. */
export function canDrop(dest: string): boolean {
  if (!dragged) return false;
  // Not into one of the dragged folders (or below it), not back where they came from.
  const intoSelf = dragged.some(
    (i) => i.location === dest || dest.startsWith(`${i.location}/`) || dest.startsWith(`${i.location}\\`),
  );
  return !intoSelf && !dragged.every((i) => parentOf(i.location) === dest);
}

/** Call from `dragover`; returns whether `dest` accepts the drop. */
export function dragOver(e: DragEvent, dest: string): boolean {
  if (!canDrop(dest)) return false;
  e.preventDefault();
  if (e.dataTransfer) e.dataTransfer.dropEffect = e.shiftKey ? 'move' : 'copy';
  return true;
}

export function drop(e: DragEvent, dest: string) {
  if (!canDrop(dest)) return;
  e.preventDefault();
  e.stopPropagation();
  const items = dragged!;
  dragged = null;
  void app.transfer(e.shiftKey ? 'move' : 'copy', items, dest);
}
