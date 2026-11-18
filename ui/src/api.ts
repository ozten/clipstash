// Typed wrappers over the Tauri commands (design section 9.1).
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type ClipKind = "text" | "image" | "file";
export type Filter = ClipKind | "snippet" | "all";

export interface Clip {
  id: number;
  kind: ClipKind;
  preview: string;
  isUrl: boolean;
  byteSize: number;
  appName: string | null;
  appBundle: string | null;
  createdAt: number;
  lastUsed: number;
  hasRich: boolean;
}

export interface ClipDetail extends Clip {
  text?: string;
  charCount?: number;
  imageUrl?: string;
  paths?: string[];
}

export interface Snippet {
  id: number;
  collectionId: number;
  name: string;
  keyword: string | null;
  body: string;
  kind: "text" | "image";
  hasBlob: boolean;
  originApp: string | null;
  useCount: number;
  createdAt: number;
  updatedAt: number;
  imageUrl?: string;
}

export interface SnippetDraft {
  collectionId: number;
  name: string;
  keyword: string | null;
  body: string;
}

export type SnippetPatch = Partial<SnippetDraft>;

export interface Collection {
  id: number;
  name: string;
  sortOrder: number;
  count: number;
}

export type Hit = { type: "clip"; item: Clip } | { type: "snippet"; item: Snippet };
export type Target = { clip: number } | { snippet: number };
export type Retention = "24h" | "7d" | "1m" | "3m";

export interface Settings {
  captureEnabled: boolean;
  onboardingDone: boolean;
  captureText: boolean;
  captureImages: boolean;
  captureFiles: boolean;
  retentionText: Retention;
  retentionImages: Retention;
  retentionFiles: Retention;
  ignoreApps: string[];
  maxTextBytes: number;
  maxImageBytes: number;
  maxRichBytes: number;
  hotkey: string;
  mergeEnabled: boolean;
  mergeSeparator: "newline" | "space" | "none";
  sound: boolean;
  launchAtLogin: boolean;
  lastCollectionId: number | null;
}

export type CaptureState = "off" | "blocked" | "paused" | "capturing";
export type AccessBehavior = "unavailable" | "default" | "ask" | "alwaysAllow" | "alwaysDeny";

export interface Status {
  state: CaptureState;
  pausedUntil: number | null;
  accessBehavior: AccessBehavior;
  accessibility: boolean;
  accessibilityPaneName: string;
  osVersion: string;
  mergeTapActive: boolean;
  launchAtLogin: boolean;
  clipCount: number;
  snippetCount: number;
}

export interface PromoteDefaults {
  name: string;
  collectionId: number;
  keyword: string;
  body: string;
  imageUrl: string | null;
}

export interface ImportReport {
  collection: string;
  collectionId: number;
  created: number;
  updated: number;
  conflicts: string[];
}

export const api = {
  search: (query: string, filter: Filter, limit: number) => invoke<Hit[]>("search", { query, filter, limit }),
  getClip: (id: number) => invoke<ClipDetail>("get_clip", { id }),
  /** Resolves to false for the copy-only fallback (no Accessibility). */
  paste: (target: Target, plain: boolean) => invoke<boolean>("paste", { target, plain }),
  copy: (target: Target, plain = false) => invoke<void>("copy", { target, plain }),
  deleteClip: (id: number) => invoke<void>("delete_clip", { id }),
  clearHistory: (scope: "5m" | "15m" | "all") => invoke<number>("clear_history", { scope }),
  hideViewer: () => invoke<void>("hide_viewer"),
  openLibrary: () => invoke<void>("open_library"),
  openSettings: (onboarding = false) => invoke<void>("open_settings", { onboarding }),

  promoteDefaults: (clipId: number) => invoke<PromoteDefaults>("promote_defaults", { clipId }),
  promote: (clipId: number, draft: SnippetDraft) => invoke<Snippet>("promote", { clipId, draft }),

  listCollections: () => invoke<Collection[]>("list_collections"),
  createCollection: (name: string) => invoke<Collection>("create_collection", { name }),
  renameCollection: (id: number, name: string) => invoke<void>("rename_collection", { id, name }),
  deleteCollection: (id: number) => invoke<void>("delete_collection", { id }),
  reorderCollections: (ids: number[]) => invoke<void>("reorder_collections", { ids }),

  listSnippets: (collectionId: number | null, query: string) =>
    invoke<Snippet[]>("list_snippets", { collectionId, query }),
  getSnippet: (id: number) => invoke<Snippet>("get_snippet", { id }),
  createSnippet: (draft: SnippetDraft) => invoke<Snippet>("create_snippet", { draft }),
  updateSnippet: (id: number, patch: SnippetPatch) => invoke<Snippet>("update_snippet", { id, patch }),
  deleteSnippet: (id: number) => invoke<void>("delete_snippet", { id }),
  duplicateSnippet: (id: number) => invoke<Snippet>("duplicate_snippet", { id }),
  exportCollection: (id: number, path: string) => invoke<number>("export_collection", { id, path }),
  importCollection: (path: string, mode: "skip" | "overwrite") =>
    invoke<ImportReport>("import_collection", { path, mode }),

  getSettings: () => invoke<Settings>("get_settings"),
  defaultSettings: () => invoke<Settings>("default_settings"),
  setSettings: (settings: Settings) => invoke<Settings>("set_settings", { settings }),
  getStatus: () => invoke<Status>("get_status"),
  setPaused: (mode: "pause" | "5m" | "resume") => invoke<void>("set_paused", { mode }),
  requestClipboardAccess: () => invoke<AccessBehavior>("request_clipboard_access"),
  requestAccessibility: () => invoke<boolean>("request_accessibility"),
  openPrivacyPane: (pane: "pasteboard" | "accessibility" | "inputMonitoring") =>
    invoke<boolean>("open_privacy_pane", { pane }),
};

export type AppEvent =
  | "clip-added"
  | "clip-removed"
  | "snippets-changed"
  | "status-changed"
  | "settings-changed"
  | "viewer-shown"
  | "show-onboarding";

export function on(event: AppEvent, handler: () => void): Promise<UnlistenFn> {
  return listen(event, () => handler());
}
