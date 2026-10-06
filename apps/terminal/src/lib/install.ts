import { useSyncExternalStore } from 'react';

/**
 * Browser install prompt (Chrome / Edge on desktop and Android). The event fires
 * once, early, so it is captured at module load and handed to whichever button
 * asks for it; after `prompt()` the browser does not offer it again.
 */
interface InstallPromptEvent extends Event {
  prompt: () => Promise<unknown>;
}

let pending: InstallPromptEvent | null = null;
const listeners = new Set<() => void>();
const notify = () => listeners.forEach((l) => l());

if (typeof window !== 'undefined') {
  window.addEventListener('beforeinstallprompt', (e) => {
    e.preventDefault();
    pending = e as InstallPromptEvent;
    notify();
  });
  window.addEventListener('appinstalled', () => {
    pending = null;
    notify();
  });
}

/** Whether the browser currently offers to install the terminal. */
export function useInstallPrompt(): boolean {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => pending !== null,
    () => false,
  );
}

/** Shows the browser's install dialog (no-op when none is offered). */
export async function promptInstall(): Promise<void> {
  const e = pending;
  if (!e) return;
  pending = null;
  notify();
  await e.prompt().catch(() => undefined);
}

/** Registers the (pass-through) service worker Chrome wants before it offers to install. */
export function registerServiceWorker(): void {
  if (!import.meta.env.PROD || typeof navigator === 'undefined' || !('serviceWorker' in navigator)) return;
  navigator.serviceWorker.register('/sw.js', { scope: '/' }).catch(() => undefined);
}
