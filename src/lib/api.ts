import { Channel, invoke } from "@tauri-apps/api/core";

export type Mirror = "osudirect" | "catboy" | "sayobot" | "nerinyan";

export interface Settings {
  lazerPath: string | null;
  mirrors: Mirror[];
  concurrency: number;
  keepOsz: boolean;
  noTrim: boolean;
  fullOnMismatch: boolean;
  theme: "dark" | "light";
  manualDir: string | null;
  replaceWarningSeen: boolean;
  language: "ru" | "en" | null;
  checkInstalled: boolean;
  exportDir: string | null;
  version: number;
}

export interface LazerStatus {
  found: boolean;
  path: string | null;
  running: boolean;
}

export interface RecentEntry {
  id: number;
  name: string;
  uploader: string;
  cover: string | null;
  diffs: number;
  sets: number;
  fetchedAt: number;
}

export interface Bucket {
  star: number;
  diffs: number;
  sets: number;
}

export interface CollectionView {
  info: { id: number; name: string; uploader: string; beatmapCount: number };
  fetchedAt: number;
  fromCache: boolean;
  buckets: Bucket[];
  stdDiffs: number;
  stdSets: number;
  setMasks: number[];
  cover: string | null;
  fromDb: boolean;
}

export interface LookupProgress {
  done: number;
  total: number;
  found: number;
  etaSecs: number | null;
}

export type LoadProgress = { stage: "info" } | { stage: "beatmaps"; loaded: number; total: number };

export interface PreviewRow {
  setId: number;
  artist: string;
  title: string;
  version: string;
  sr: number;
}

export type SetStatus =
  | "pending"
  | "downloading"
  | "trimming"
  | "queued"
  | "importing"
  | "done"
  | "skipped"
  | "failed"
  | "cancelled";

export interface SetRow {
  idx: number;
  setId: number;
  artist: string;
  title: string;
  diffs: number;
  status: SetStatus;
  progress: number;
  mirror: string | null;
  error: string | null;
  note: string | null;
  mismatch: boolean;
}

export type LazerState = "manual" | "notRunning" | "ready";

export interface Summary {
  total: number;
  pending: number;
  downloading: number;
  trimming: number;
  queued: number;
  importing: number;
  done: number;
  skipped: number;
  failed: number;
  cancelled: number;
  doneDiffs: number;
  bytes: number;
  speedBps: number;
  etaSecs: number | null;
  paused: boolean;
  pauseReason: string | null;
  lazer: LazerState | null;
  finished: boolean;
  cancelling: boolean;
  exportPath: string | null;
  exportDiffs: number;
  exportCollections: { name: string; count: number }[];
  manualDir: string | null;
}

export interface Progress {
  jobId: number;
  collectionName: string;
  rows: SetRow[];
  summary: Summary;
}

export interface ScanSummary {
  found: boolean;
  path: string | null;
  sets: number;
  diffs: number;
}

export interface InstalledCount {
  diffs: number;
  sets: number;
}

export interface ExportInfo {
  dir: string;
  defaultDir: string;
  collections: { name: string; count: number }[];
}

export interface DbCollection {
  name: string;
  count: number;
}

export const api = {
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<Settings>("save_settings", { settings }),
  lazerStatus: () => invoke<LazerStatus>("lazer_status"),
  detectedLazerPaths: () => invoke<string[]>("detected_lazer_paths"),
  launchLazer: () => invoke<void>("launch_lazer"),
  recentCollections: () => invoke<RecentEntry[]>("recent_collections"),
  loadCollection: (input: string, refresh: boolean, onProgress: (p: LoadProgress) => void) => {
    const ch = new Channel<LoadProgress>();
    ch.onmessage = onProgress;
    return invoke<CollectionView>("load_collection", { input, refresh, onProgress: ch });
  },
  selectionPreview: (collectionId: number, buckets: number[]) =>
    invoke<PreviewRow[]>("selection_preview", { collectionId, buckets }),
  startJob: (collectionId: number, buckets: number[], manualDir: string | null) =>
    invoke<number>("start_job", { collectionId, buckets, manualDir }),
  jobSnapshot: () => invoke<Progress | null>("job_snapshot"),
  pauseJob: () => invoke<void>("pause_job"),
  resumeJob: () => invoke<void>("resume_job"),
  cancelJob: () => invoke<void>("cancel_job"),
  retryFailed: () => invoke<boolean>("retry_failed"),
  clearCache: () => invoke<void>("clear_cache"),
  clearHistory: () => invoke<void>("clear_history"),
  historySize: () => invoke<number>("history_size"),
  openFolder: (path: string) => invoke<void>("open_folder", { path }),
  readCollectionDb: (path: string) => invoke<DbCollection[]>("read_collection_db", { path }),
  loadDbCollection: (path: string, index: number, onProgress: (p: LookupProgress) => void) => {
    const ch = new Channel<LookupProgress>();
    ch.onmessage = onProgress;
    return invoke<CollectionView>("load_db_collection", { path, index, onProgress: ch });
  },
  cancelDbLookup: () => invoke<void>("cancel_db_lookup"),
  scanLazer: () => invoke<ScanSummary>("scan_lazer"),
  exportInfo: () => invoke<ExportInfo>("export_info"),
  clearExport: () => invoke<void>("clear_export"),
  isInstalled: () => invoke<boolean>("is_installed"),
  installedCount: (collectionId: number, buckets: number[]) =>
    invoke<InstalledCount>("installed_count", { collectionId, buckets }),
};

export function errorText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return String(e);
}
