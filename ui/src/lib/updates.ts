// In-app updates from the GitHub release feed (see crates/porter-gui/tauri.conf.json).

import { inTauri } from './api';
import { app, message } from './state.svelte';

/** Checks for a newer release and offers to install it. Automatic checks stay quiet
 * unless there is an update; manual ones report every outcome. */
export async function checkForUpdates(manual: boolean) {
  if (!inTauri) {
    if (manual) app.notify('Updates are checked in the desktop app');
    return;
  }
  try {
    const { check } = await import('@tauri-apps/plugin-updater');
    const update = await check();
    if (!update) {
      if (manual) app.notify('Porter is up to date');
      return;
    }
    const notes = update.body?.split('\n').find((line) => line.trim())?.trim();
    const answer = await app.ask({
      type: 'choice',
      title: `Porter ${update.version} is available`,
      message: `You have ${update.currentVersion}.${notes ? ` ${notes}` : ''} Install it now? Porter restarts afterwards; running transfers are stopped.`,
      choices: [
        { label: 'Later', value: 'later' },
        { label: 'Install and restart', value: 'install', primary: true },
      ],
    });
    if (answer !== 'install') return;
    app.notify(`Downloading Porter ${update.version}…`);
    await update.downloadAndInstall();
    const { relaunch } = await import('@tauri-apps/plugin-process');
    await relaunch();
  } catch (e) {
    if (manual) app.notify(`Couldn't check for updates: ${message(e)}`, true);
  }
}

export async function showVersion() {
  const version = inTauri
    ? await (await import('@tauri-apps/api/app')).getVersion()
    : 'development build (browser)';
  app.notify(`Porter ${version}`);
}
