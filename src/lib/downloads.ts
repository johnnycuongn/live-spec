// Download progress lives outside React components so closing Settings
// doesn't lose track of a multi-GB download still running in Rust.
import { useSyncExternalStore } from "react";
import { api, errorMessage, type UiEvent } from "./ipc";

export interface DownloadState {
  downloaded: number;
  total: number | null;
  error: string | null;
  finished: boolean;
}

let downloads: Record<string, DownloadState> = {};
const listeners = new Set<() => void>();

function update(key: string, patch: Partial<DownloadState>) {
  const prev = downloads[key] ?? { downloaded: 0, total: null, error: null, finished: false };
  downloads = { ...downloads, [key]: { ...prev, ...patch } };
  listeners.forEach((l) => l());
}

function handler(key: string, onFinished: () => void) {
  return (ev: UiEvent) => {
    if (ev.type === "progress") update(key, { downloaded: ev.downloaded, total: ev.total });
    else if (ev.type === "done") {
      update(key, { finished: true });
      onFinished();
    } else if (ev.type === "error") update(key, { error: ev.message, finished: true });
  };
}

export function startDownload(key: "runtime" | (string & {}), onFinished: () => void) {
  update(key, { error: null, finished: false });
  const onEvent = handler(key, onFinished);
  const call = key === "runtime" ? api.installRuntime(onEvent) : api.downloadModel(key, onEvent);
  call.catch((e) => update(key, { error: errorMessage(e), finished: true }));
}

export function pauseDownload(key: string) {
  void api.cancelDownload(key);
  update(key, { finished: true });
}

export function useDownloads() {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => downloads,
  );
}

export const formatBytes = (n: number) =>
  n >= 1e9 ? `${(n / 1e9).toFixed(2)} GB` : n >= 1e6 ? `${(n / 1e6).toFixed(0)} MB` : `${(n / 1e3).toFixed(0)} KB`;
