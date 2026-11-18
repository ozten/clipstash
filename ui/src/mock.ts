// Browser preview: when the UI is opened outside Tauri (`npm run dev`, then
// http://localhost:5173/viewer.html) a fake backend is installed so the three
// windows can be worked on without building the app. Development builds only.

type Args = Record<string, any>;

function install() {
  const now = Date.now();
  const min = 60_000;
  const collections = [
    { id: 1, name: "Email", sortOrder: 0 },
    { id: 2, name: "SQL", sortOrder: 1 },
    { id: 3, name: "Shell", sortOrder: 2 },
  ];
  let nextId = 100;
  const snippets: Args[] = [
    { id: 1, collectionId: 1, name: "Signature", keyword: ";sig", body: "Thanks,\n{cursor}\n— Sam", kind: "text", hasBlob: false, originApp: "Mail", useCount: 14, createdAt: now, updatedAt: now },
    { id: 2, collectionId: 1, name: "Address", keyword: null, body: "1 Example Street\nSpringfield", kind: "text", hasBlob: false, originApp: null, useCount: 3, createdAt: now, updatedAt: now },
    { id: 3, collectionId: 1, name: "Standup template", keyword: ";su", body: "Yesterday:\nToday: {cursor}\nBlockers:\n({date:%A %d %B})", kind: "text", hasBlob: false, originApp: null, useCount: 8, createdAt: now, updatedAt: now },
    { id: 4, collectionId: 2, name: "Recent clips", keyword: ";rc", body: "SELECT id, kind, substr(text, 1, 60)\nFROM clip\nORDER BY last_used DESC\nLIMIT 20;", kind: "text", hasBlob: false, originApp: "Terminal", useCount: 5, createdAt: now, updatedAt: now },
    { id: 5, collectionId: 3, name: "Reset pasteboard grant", keyword: null, body: "tccutil reset Pasteboard com.clipstash.app.dev", kind: "text", hasBlob: false, originApp: null, useCount: 1, createdAt: now, updatedAt: now },
  ];
  const clip = (id: number, kind: string, text: string, appName: string, ago: number, extra: Args = {}) => ({
    id, kind, text, preview: text.slice(0, 300), isUrl: /^https?:\/\/\S+$/.test(text), byteSize: text.length,
    appName, appBundle: `/Applications/${appName}.app`, createdAt: now - ago, lastUsed: now - ago, hasRich: false, ...extra,
  });
  const clips: Args[] = [
    clip(9, "text", "https://example.com/docs/getting-started?ref=clipboard", "Safari", 2 * min),
    clip(8, "text", "SELECT item, app FROM clipboard\nWHERE ts > strftime('%s','now','-1 day')\nORDER BY ts DESC;", "Terminal", 6 * min),
    clip(7, "image", "[image 1280×720]", "Preview", 20 * min, { byteSize: 482_113 }),
    clip(6, "file", "~/Downloads/report.pdf, ~/Downloads/appendix.xlsx", "Finder", 45 * min, { paths: ["/Users/sam/Downloads/report.pdf", "/Users/sam/Downloads/appendix.xlsx"] }),
    clip(5, "text", "The quick brown fox jumps over the lazy dog. ".repeat(12).trim(), "Notes", 3 * 60 * min, { hasRich: true }),
    clip(4, "text", "cargo test -p clipstash-core", "Terminal", 5 * 60 * min),
    clip(3, "text", "Meeting moved to Thursday 14:00, room 4B", "Mail", 26 * 60 * min),
    clip(2, "text", "fn main() {\n    println!(\"hello\");\n}", "Zed", 3 * 24 * 60 * min),
  ];
  let settings: Args = {
    captureEnabled: true, onboardingDone: true, captureText: true, captureImages: true, captureFiles: true,
    retentionText: "1m", retentionImages: "7d", retentionFiles: "7d",
    ignoreApps: ["com.1password.1password", "com.bitwarden.desktop", "org.keepassxc.keepassxc"],
    maxTextBytes: 1 << 20, maxImageBytes: 20 << 20, maxRichBytes: 256 << 10, hotkey: "Alt+Super+KeyC",
    mergeEnabled: false, mergeSeparator: "newline", sound: true, launchAtLogin: false, lastCollectionId: 1,
  };
  const params = new URLSearchParams(location.search);
  const status = () => ({
    state: params.get("state") ?? (settings.captureEnabled ? "capturing" : "off"), pausedUntil: null,
    accessBehavior: params.get("access") ?? "alwaysAllow", accessibility: params.get("ax") !== "0",
    accessibilityPaneName: "Accessibility", osVersion: "26.6.2", mergeTapActive: false, launchAtLogin: false,
    clipCount: clips.length, snippetCount: snippets.length,
  });
  const cols = () => collections.map((c) => ({ ...c, count: snippets.filter((s) => s.collectionId === c.id).length }));
  const match = (hay: string, q: string) => q.toLowerCase().split(/\s+/).every((t) => hay.toLowerCase().includes(t));
  const PIXEL = "data:image/svg+xml," + encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="640" height="360"><defs><linearGradient id="g" x2="1" y2="1"><stop stop-color="#4876ff"/><stop offset="1" stop-color="#22be9a"/></linearGradient></defs><rect width="640" height="360" fill="url(#g)"/></svg>');

  const handlers: Record<string, (a: Args) => unknown> = {
    search: ({ query, filter }) => {
      const q = (query as string).trim();
      const snips = q || filter === "snippet" ? snippets.filter((s) => !q || match(`${s.name} ${s.keyword ?? ""} ${s.body}`, q)) : [];
      const cl = clips.filter((c) => (filter === "all" || filter === c.kind) && (!q || match(`${c.text} ${c.appName}`, q)));
      return [
        ...(filter === "all" || filter === "snippet" ? snips.map((item) => ({ type: "snippet", item })) : []),
        ...(filter === "snippet" ? [] : cl.map((item) => ({ type: "clip", item }))),
      ];
    },
    get_clip: ({ id }) => {
      const c = clips.find((x) => x.id === id)!;
      return { ...c, charCount: c.text.length, imageUrl: c.kind === "image" ? PIXEL : undefined };
    },
    paste: () => params.get("ax") !== "0",
    delete_clip: ({ id }) => void clips.splice(clips.findIndex((c) => c.id === id), 1),
    clear_history: () => clips.splice(0).length,
    promote_defaults: ({ clipId }) => {
      const c = clips.find((x) => x.id === clipId)!;
      return { name: c.text.split("\n")[0].slice(0, 60), collectionId: 1, keyword: "", body: c.text, imageUrl: c.kind === "image" ? PIXEL : null };
    },
    promote: ({ draft }) => (snippets.push({ id: nextId++, kind: "text", hasBlob: false, originApp: "Safari", useCount: 0, createdAt: now, updatedAt: now, ...draft }), snippets.at(-1)),
    list_collections: cols,
    create_collection: ({ name }) => (collections.push({ id: nextId++, name, sortOrder: collections.length }), { ...collections.at(-1)!, count: 0 }),
    rename_collection: ({ id, name }) => void (collections.find((c) => c.id === id)!.name = name),
    delete_collection: ({ id }) => void collections.splice(collections.findIndex((c) => c.id === id), 1),
    reorder_collections: ({ ids }) => void collections.sort((a, b) => ids.indexOf(a.id) - ids.indexOf(b.id)),
    list_snippets: ({ collectionId, query }) =>
      snippets.filter((s) => (collectionId == null || s.collectionId === collectionId) && (!query || match(`${s.name} ${s.body}`, query))),
    create_snippet: ({ draft }) => (snippets.push({ id: nextId++, kind: "text", hasBlob: false, originApp: null, useCount: 0, createdAt: now, updatedAt: now, ...draft }), snippets.at(-1)),
    update_snippet: ({ id, patch }) => Object.assign(snippets.find((s) => s.id === id)!, patch),
    delete_snippet: ({ id }) => void snippets.splice(snippets.findIndex((s) => s.id === id), 1),
    duplicate_snippet: ({ id }) => {
      const s = snippets.find((x) => x.id === id)!;
      snippets.push({ ...s, id: nextId++, name: `${s.name} copy`, keyword: null });
      return snippets.at(-1);
    },
    get_settings: () => settings,
    default_settings: () => settings,
    set_settings: ({ settings: s }) => (settings = s),
    get_status: status,
    request_clipboard_access: () => status().accessBehavior,
    request_accessibility: () => status().accessibility,
    "plugin:dialog|ask": () => true,
    "plugin:event|listen": () => 0,
  };

  let callbackId = 0;
  (window as any).__TAURI_INTERNALS__ = {
    transformCallback: () => ++callbackId,
    invoke: async (cmd: string, args: Args = {}) => {
      const handler = handlers[cmd];
      const result = handler ? handler(args) : null;
      return result === undefined ? null : structuredClone(result);
    },
  };
  (window as any).__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
}

if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) install();

export {};
