# Clipboard History + Snippets — Design Doc

Status: Draft v2 (adds menu-bar shell and macOS permission model)
Date: 2026-11-17
Working title: `clipstash`

## 1. Summary

A macOS menu-bar utility that records what you copy, lets you search and re-paste it from a keyboard-driven viewer, and lets you promote any clip into a permanent, organized snippet library.

Clipboard history matches Alfred's Powerpack feature. The snippet library goes further than Alfred's: it gets a dedicated UI with collections, editing, and placeholders.

### Assumptions (change these if wrong)

| Assumption | Reason |
|---|---|
| macOS only for v1 | The reference feature is Alfred's; the capture and paste layers are OS-specific |
| Tauri 2: Rust core, TypeScript UI | Fits a Rust + TypeScript stack; small native binary |
| SQLite for all storage | Local, queryable from the CLI, FTS5 for search |
| Single user, single machine | History is private and local; no sync in v1 |
| Menu-bar app, no Dock icon | Tray icon + Accessory activation policy |
| Notarized Developer ID build, not the Mac App Store | Synthesized paste does not work inside the App Sandbox (section 10.4) |
| Minimum macOS 14; must behave correctly on 15.4+ and 27.2+ | Pasteboard privacy API starts at 15.4; its developer flag is removed in 27.2 |

## 2. Goals and non-goals

**Goals**

1. Capture text, links, images and file references as they are copied.
2. Open a viewer with a global hotkey, filter by typing, paste with Return.
3. Never store secrets: honor ignore lists and concealed-content markers.
4. Expire history automatically, with a separate retention period per clip type.
5. Promote a clip to a snippet in one keystroke, then manage snippets in a library UI.
6. Keep everything inspectable: plain SQLite files and a CLI.

**Non-goals for v1**

- Cross-device sync of history.
- Windows or Linux support.
- OCR or search inside images.
- Paste queue / multi-paste stack.
- Type-to-expand snippet keywords (planned for v2, see section 8.5).

## 3. Feature list

### Clipboard history (Alfred parity)

| # | Feature | Behavior |
|---|---|---|
| H0 | Menu-bar app | Lives in the menu bar with no Dock icon; tray menu shows capture status |
| H1 | Off by default | Capture starts only after the user enables it and grants permissions |
| H2 | Clip types | Text, images, file lists; each has its own enable toggle |
| H3 | Retention | Per type: 24 hours, 7 days, 1 month, 3 months |
| H4 | Source tracking | Store source app name and bundle path with each clip |
| H5 | Ignore apps | Clips from listed apps are dropped; ships with common password managers listed |
| H6 | Concealed content | Drop anything the source marks as concealed or transient |
| H7 | Size cap | Drop text over a configurable size |
| H8 | Viewer | Global hotkey (default ⌥⌘C); type to filter; Return pastes into the frontmost app |
| H9 | Copy back | ⌘C in the viewer puts the clip on the clipboard without pasting |
| H10 | Delete one | Fn+Delete removes the highlighted clip |
| H11 | Clear | Clear the last 5 minutes, the last 15 minutes, or everything |
| H12 | Merge | Optional: hold ⌘ and double-tap C to append the selection to the previous clip |
| H13 | Promote | ⌘S saves the highlighted clip as a snippet |

### Snippet library (new)

| # | Feature | Behavior |
|---|---|---|
| S1 | Promote flow | ⌘S in the viewer opens a small form pre-filled from the clip |
| S2 | Collections | Snippets are grouped into named collections |
| S3 | Library window | Three panes: collections, snippet list, editor |
| S4 | Search | Snippets appear in the viewer alongside history, ranked above it |
| S5 | Placeholders | `{date}`, `{time}`, `{clipboard}`, `{cursor}` resolved at paste time |
| S6 | No expiry | Snippets are never touched by retention or clear |
| S7 | Import / export | JSON per collection, for backup and version control |

## 4. Architecture

```
┌──────────────────────── Rust core (Tauri) ────────────────────────┐
│                                                                    │
│  PasteboardWatcher ──► CaptureFilter ──► Store (SQLite + blobs)    │
│   (poll changeCount)    (ignore apps,          ▲          ▲        │
│                          concealed, size)      │          │        │
│                                                │          │        │
│  HotkeyManager ──► ViewerController ──► Search ┘   RetentionJob    │
│                          │                                         │
│                          ▼                                         │
│                     PasteEngine (write pasteboard, synthesize ⌘V)  │
│                                                                    │
└───────────────▲──────────────────────────────────▲────────────────┘
                │ Tauri commands + events          │
      ┌─────────┴─────────┐              ┌─────────┴─────────┐
      │ Viewer panel (TS) │              │ Library window(TS)│
      └───────────────────┘              └───────────────────┘
                         CLI (Rust, same Store crate)
```

Crate layout:

- `core/` — store, search, filters, retention, placeholder expansion. No UI or OS dependencies, fully unit-testable.
- `platform-macos/` — pasteboard, frontmost app, event synthesis, hotkeys.
- `app/` — Tauri shell, tray icon, commands, windows.
- `cli/` — thin binary over `core/`.
- `ui/` — TypeScript front end for the viewer and library.

### 4.1 Menu-bar shell

- Tray icon via Tauri's `tray-icon` feature; `ActivationPolicy::Accessory` so the app has no Dock icon and is absent from ⌘Tab.
- Launch at login via `SMAppService` (user opt-in).
- The library window is a normal window. While it is open the app may switch to the Regular activation policy so it can be reached with ⌘Tab, then switch back on close.

Tray menu:

| Item | Notes |
|---|---|
| Status line | "Capturing", "Paused", or "Clipboard access needed…" (opens onboarding) |
| Open Clipboard History | Same as the global hotkey |
| Open Snippet Library | |
| Pause / Pause for 5 minutes | |
| Clear ▸ | Last 5 minutes, last 15 minutes, all |
| Settings… | |
| Quit | |

The tray icon changes when capture is paused or blocked, so a silent loss of access is visible at a glance.

## 5. Capture pipeline

### 5.1 Detecting copies

macOS has no notification for clipboard changes. The watcher polls `NSPasteboard.general.changeCount` on a timer and reads the pasteboard only when the count moves.

- Poll interval: 250 ms while the user is active, 1 s after 60 s of idle.
- Cost per poll is one integer read, so this is cheap.

**Access gate.** On macOS 15.4+, before reading contents the watcher checks `NSPasteboard.general.accessBehavior`:

| State | Watcher behavior |
|---|---|
| `alwaysAllow` | Read and capture |
| `default` or `ask` | Do not read. A background read here raises a system alert on every copy. Set status to "Clipboard access needed" |
| `alwaysDeny` | Do not read; reads would be refused silently. Same status |
| API unavailable (macOS 14) | Read and capture; there is no gate |

The state is re-checked on every poll tick that sees a change, so capture resumes on its own once the user grants access. See section 10 for the full permission model.

### 5.2 Reading a clip

On change, classify by the first matching type:

| Priority | Pasteboard type | Stored as |
|---|---|---|
| 1 | File URLs | `kind = file`, JSON array of paths; the files themselves are not copied |
| 2 | Image (PNG, TIFF) | `kind = image`, blob file on disk, PNG-encoded |
| 3 | Plain text | `kind = text`; links are text with `is_url = 1` |

Rich text (RTF, HTML) is kept as an optional secondary payload so a clip can be pasted with or without formatting. Default paste is plain text.

### 5.3 Filters, applied in order

1. **Own writes.** The paste engine records the change count it produced; the watcher skips that count.
2. **Type disabled.** Drop if the clip type's toggle is off.
3. **Concealed or transient.** Drop if the pasteboard carries `org.nspasteboard.ConcealedType`, `org.nspasteboard.TransientType` or `org.nspasteboard.AutoGeneratedType`.
4. **Ignored app.** Drop if the frontmost app's bundle id is on the ignore list.
5. **Size cap.** Drop text over the limit (default 1 MB) and images over a separate limit (default 20 MB).
6. **Duplicate.** If the content hash matches an existing clip, update that row's timestamp instead of inserting.

### 5.4 Source app

The pasteboard does not say who wrote to it. Use `NSWorkspace.frontmostApplication` at the moment the change is seen. This is a heuristic: a background app writing to the clipboard will be attributed to whatever is in front. The ignore list inherits that limit, the same as Alfred's.

### 5.5 Merge (H12)

Detecting "⌘ held, C tapped twice" needs to see key events, which means a listen-only `CGEventTap`. A listen-only tap is covered by Input Monitoring, and the Accessibility grant already includes listening, so merge adds no extra prompt once paste is set up.

- On a double tap within 400 ms, wait for the pasteboard change, then append the new text to the previous text clip, joined by a configurable separator (newline, space, or none).
- Play a short sound to confirm.
- Merge is off by default, and the event tap is only installed when it is on.

Rejected alternative: inferring a double copy from two rapid change-count bumps. Many apps bump the count more than once per copy, so this misfires.

## 6. Storage

Two SQLite files under `~/Library/Application Support/clipstash/`:

| File | Contents | Lifetime |
|---|---|---|
| `history.db` + `blobs/` | Clips | Disposable; pruned by retention |
| `snippets.db` | Collections and snippets | Durable; safe to back up or sync |

Splitting them keeps the durable data small and means history can be wiped or excluded from backups without touching snippets.

### 6.1 `history.db`

```sql
CREATE TABLE clip (
  id          INTEGER PRIMARY KEY,
  kind        TEXT NOT NULL CHECK (kind IN ('text','image','file')),
  text        TEXT,              -- text content, or JSON array of paths for files
  rich        BLOB,              -- optional RTF/HTML payload
  blob_path   TEXT,              -- relative path under blobs/ for images
  is_url      INTEGER NOT NULL DEFAULT 0,
  byte_size   INTEGER NOT NULL,
  hash        TEXT NOT NULL UNIQUE,   -- BLAKE3 of the canonical payload
  app_name    TEXT,
  app_bundle  TEXT,
  created_at  INTEGER NOT NULL,       -- unix ms, first seen
  last_used   INTEGER NOT NULL        -- unix ms, bumped on re-copy or paste
);
CREATE INDEX clip_last_used ON clip(last_used DESC);
CREATE INDEX clip_kind ON clip(kind);

CREATE VIRTUAL TABLE clip_fts USING fts5(
  text, app_name, content='clip', content_rowid='id',
  tokenize='trigram'
);
```

Trigram tokenizing gives substring matching, which is what people expect when filtering clips ("typing any part of it finds it").

### 6.2 `snippets.db`

```sql
CREATE TABLE collection (
  id         INTEGER PRIMARY KEY,
  name       TEXT NOT NULL UNIQUE,
  sort_order INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE snippet (
  id            INTEGER PRIMARY KEY,
  collection_id INTEGER NOT NULL REFERENCES collection(id) ON DELETE CASCADE,
  name          TEXT NOT NULL,
  keyword       TEXT,             -- reserved for v2 auto-expansion
  body          TEXT NOT NULL,    -- may contain placeholders
  kind          TEXT NOT NULL DEFAULT 'text',   -- 'text' | 'image'
  blob          BLOB,             -- image snippets carry their own bytes
  origin_app    TEXT,             -- where the clip came from, if promoted
  use_count     INTEGER NOT NULL DEFAULT 0,
  created_at    INTEGER NOT NULL,
  updated_at    INTEGER NOT NULL
);
CREATE UNIQUE INDEX snippet_keyword ON snippet(keyword) WHERE keyword IS NOT NULL;

CREATE VIRTUAL TABLE snippet_fts USING fts5(
  name, keyword, body, content='snippet', content_rowid='id',
  tokenize='trigram'
);
```

Promotion **copies** the clip into `snippets.db`. There is no foreign key back to history, so pruning or clearing history can never break a snippet.

### 6.3 Retention

A job runs at launch and hourly:

```sql
DELETE FROM clip WHERE kind = ?1 AND last_used < ?2;
```

It then removes orphaned files in `blobs/`. "Clear last 5/15 minutes" filters on `created_at` instead, so a recently re-used old clip is not caught by accident.

## 7. Viewer

### 7.1 Window

A borderless, non-activating floating panel (`NSPanel`). The app underneath must stay frontmost so paste lands in it. Stock Tauri windows activate the app, so the window is converted with the `tauri-nspanel` plugin (non-activating style mask), the same approach existing Tauri menu-bar apps use.

The panel opens from the global hotkey (centered) or from the tray icon (anchored under it).

### 7.2 Layout

```
┌───────────────────────────────────────────────────────┐
│ 🔍 filter…                                 [All ▾]     │
├───────────────────────────┬───────────────────────────┤
│ ★ Signature (Snippets)    │                           │
│   https://example.com/…   │   Full preview of the     │
│ ▸ SELECT item, app FROM…  │   highlighted clip        │
│   [image 1280×720]        │                           │
│   ~/Downloads/report.pdf  │   Safari · 2 min ago      │
│                           │   412 chars               │
└───────────────────────────┴───────────────────────────┘
```

- Empty filter: history in `last_used` order.
- With a filter: snippets first, then clips, each ranked by FTS score with a recency boost.
- The type menu narrows to text, images, files, or snippets.

### 7.3 Keys

| Key | Action |
|---|---|
| ↑ / ↓ | Move selection |
| Return | Paste into the frontmost app |
| ⇧Return | Paste as plain text (when a rich payload exists) |
| ⌘C | Copy to clipboard, do not paste |
| ⌘S | Promote to snippet |
| Fn+Delete | Delete clip |
| ⌘1…⌘9 | Paste the nth visible row |
| Esc | Close |

### 7.4 Paste engine

1. Resolve placeholders if the item is a snippet.
2. Write the payload to the pasteboard and record the resulting change count.
3. Hide the panel.
4. Synthesize ⌘V with `CGEvent`.
5. Bump `last_used` (clips) or `use_count` (snippets).

Step 4 is why the app needs Accessibility permission.

**Copy-only fallback.** If Accessibility is not granted, the engine stops after step 3 and shows a brief "Copied — press ⌘V" hint. The app is fully usable this way; direct paste is an upgrade, not a requirement.

The engine's own pasteboard writes are not reads, so they are unaffected by the pasteboard access gate.

## 8. Snippet library

### 8.1 Promote flow (S1)

⌘S on a clip opens a sheet inside the viewer:

| Field | Default |
|---|---|
| Name | First line of the clip, truncated to 60 characters |
| Collection | Last used collection |
| Keyword | Empty |
| Body | The clip, editable |

Return saves and closes; the clip stays in history. Image clips can be promoted too; their body is replaced by an image preview.

### 8.2 Library window (S3)

```
┌─────────────┬──────────────────────┬──────────────────────────────┐
│ Collections │ Snippets             │ Editor                       │
│             │                      │                              │
│ All         │ Signature            │ Name     [Signature        ] │
│ Email       │ Address              │ Keyword  [;sig             ] │
│ SQL         │ Standup template     │ Body                         │
│ Shell       │ …                    │ ┌──────────────────────────┐ │
│             │                      │ │ Thanks,                  │ │
│ + New       │ 🔍 filter            │ │ {cursor}                 │ │
│             │                      │ └──────────────────────────┘ │
│             │                      │ Insert: {date} {time} …      │
│             │                      │ Used 14× · from Mail         │
└─────────────┴──────────────────────┴──────────────────────────────┘
```

- Drag snippets between collections; drag collections to reorder.
- Edits autosave on blur.
- Duplicate, delete, and "copy to clipboard" on each snippet.
- Opened from the menu-bar item, from the viewer (⌘L), or `clipstash library`.

### 8.3 Placeholders (S5)

| Token | Resolves to |
|---|---|
| `{date}` | Today's date; optional format, e.g. `{date:%Y-%m-%d}` |
| `{time}` | Current time; optional format |
| `{clipboard}` | Current clipboard text; `{clipboard:1}` is the previous clip |
| `{cursor}` | Where the caret ends up after paste |

`{cursor}` is implemented by pasting, then sending the right number of ← key events. Unknown tokens are left as literal text.

### 8.4 Import / export (S7)

One JSON file per collection:

```json
{
  "collection": "SQL",
  "snippets": [
    { "name": "Recent clips", "keyword": ";rc", "body": "SELECT …" }
  ]
}
```

Import matches on name within a collection and asks before overwriting.

### 8.5 v2: type-to-expand

Typing a keyword such as `;sig` in any app replaces it with the snippet. This needs an active event tap that watches keystrokes, plus per-app exclusions and secure-input handling. It is deliberately held back: the `keyword` column exists now so no migration is needed later.

## 9. Interfaces

### 9.1 Tauri commands (Rust → TypeScript)

```ts
type ClipKind = "text" | "image" | "file";

interface Clip {
  id: number;
  kind: ClipKind;
  preview: string;        // truncated text, or a description for images/files
  isUrl: boolean;
  byteSize: number;
  appName: string | null;
  lastUsed: number;
}

interface Snippet {
  id: number;
  collectionId: number;
  name: string;
  keyword: string | null;
  body: string;
  useCount: number;
}

search(query: string, filter: ClipKind | "snippet" | "all", limit: number):
  Promise<Array<{ type: "clip"; item: Clip } | { type: "snippet"; item: Snippet }>>;
getClip(id: number): Promise<Clip & { text?: string; imageUrl?: string }>;
paste(target: { clip: number } | { snippet: number }, plain: boolean): Promise<void>;
copy(target: { clip: number } | { snippet: number }): Promise<void>;
deleteClip(id: number): Promise<void>;
clearHistory(scope: "5m" | "15m" | "all"): Promise<number>;
promote(clipId: number, draft: Omit<Snippet, "id" | "useCount">): Promise<Snippet>;
// plus CRUD for snippets and collections, and get/set settings
```

Events pushed to the UI: `clip-added`, `clip-removed`, `snippets-changed`.

### 9.2 CLI

```
clipstash list [--kind text] [--limit 20] [--json]
clipstash search <query>
clipstash copy <id>
clipstash clear [5m|15m|all]
clipstash snippet list|add|export|import
clipstash library
```

The CLI opens the same SQLite files (WAL mode), so it works while the app is running.

## 10. Privacy and permissions

### 10.1 Permission summary

| Permission | Needed for | Required? | Where the user grants it |
|---|---|---|---|
| Paste from Other Apps (pasteboard access) | Capturing clips in the background | Yes on macOS 15.4+ when enforced; nothing to grant on 14 | Privacy & Security > Paste from Other Apps, set to Allow |
| Accessibility | Synthesized ⌘V; also covers the merge event tap | Optional (copy-only fallback) | Privacy & Security > Accessibility; on macOS 27 the list is named "Device Control and Data Access" |
| Input Monitoring | Merge event tap, only if Accessibility is not granted | Optional | Privacy & Security > Input Monitoring |

### 10.2 Pasteboard access (the critical one)

What is known:

- macOS 15.4 added `NSPasteboard.accessBehavior` with four states: default, ask, always allow, always deny. It is read-only; only the user can change it.
- Reads that are user-originated and paste-related (⌘V, Edit > Paste) are exempt. Background polling reads are what the system alert targets.
- "Allow" in the alert grants a single read. Standing access is only available from the Settings pane.
- An app appears in the Settings pane only after it has triggered the alert once.
- Under "deny", reads fail silently with no user-visible signal.
- `detect` methods on `NSPasteboard` and `NSPasteboardItem` report what kinds of data are present without reading them and without an alert.
- `tccutil reset Pasteboard <bundle-id>` resets the state, which is useful for testing.
- Through macOS 26 the alert was opt-in via the `EnablePasteboardPrivacyDeveloperPreview` user default. The macOS 27.2 release notes list that default as removed.

What is not known (resolve in M0):

- Whether macOS 27.2 enforces the alert for every app by default. No first-hand test result was found; reports conflict.
- Whether `changeCount` polling and `types` are gated. Apple does not document this.
- What a denied read returns (nil, empty, or a hang; a hang was seen on 27.0.1 with the developer flag set).

Design consequences:

1. Capture is gated on `accessBehavior` (section 5.1). The app never reads in the `default` or `ask` states outside onboarding, so it cannot spam alerts.
2. The design must work whether or not 27.2 enforces by default, because the gate reads the state instead of assuming an OS version.
3. If `changeCount` turns out to be gated, the watcher has no cheap change signal while access is missing. That only matters in the blocked state, where it is not capturing anyway.

### 10.3 Onboarding flow

Shown on first launch and whenever a required grant is missing.

1. **Explain.** One screen: what is captured, what is skipped, that data stays local.
2. **Clipboard access.** On 15.4+ when the state is not always-allow: the app performs one deliberate read so macOS shows its alert and lists the app in Settings. It then opens the pane and shows a picture of the toggle to set. The screen advances by itself when `accessBehavior` becomes always-allow.
3. **Direct paste (optional).** Request Accessibility with `AXIsProcessTrustedWithOptions`; offer "Skip, I'll paste with ⌘V". Copy names the pane by OS version.
4. **Hotkey.** Confirm or change ⌥⌘C.

Settings deep links (verify per OS version in M0):

- `x-apple.systempreferences:com.apple.preference.security?Privacy_Pasteboard`
- `x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_Pasteboard` (reported on newer versions)

Apple's stated guidance is to explain in onboarding and not jump straight from an alert to Settings, which this flow follows.

### 10.4 Distribution and signing

- **Not the Mac App Store.** `CGEvent.post` is blocked in the App Sandbox, and a clipboard manager has been rejected in review for using it. Ship a notarized Developer ID build with hardened runtime, outside the sandbox.
- **Stable signing identity.** Privacy grants are tied to the code signature. Sign every build, including development builds, with the same certificate and bundle id, or grants are lost on each rebuild. Grants have been observed to survive updates signed with the same certificate.
- **Dev vs release bundle ids.** Use a separate bundle id for development so testing with `tccutil reset` never touches the installed release build.

### 10.5 Other privacy handling

| Concern | Handling |
|---|---|
| Passwords in history | Concealed markers honored; default ignore list of password managers; capture off until enabled |
| Data at rest | Files are user-readable only (0600). No encryption in v1; rely on FileVault |
| Network | The app makes no network requests |
| Pause | Tray toggle to pause capture, with "pause for 5 minutes" |
| Secure input | When secure input is active, the paste engine and event tap stand down |
| Lost access | Tray icon and status line change; no silent failure |

## 11. Risks and open questions

1. **Pasteboard access enforcement on macOS 27.2+** (section 10.2). Highest risk. Worst case is still workable: one extra onboarding step. The unknowns are about friction, not feasibility. Resolved by M0.
2. **Non-activating panel in Tauri.** `tauri-nspanel` is the plan. Fallback: activate the app, remember the previous frontmost app, re-activate it before pasting. Slower and flickers. Covered by M0.
3. **`accessBehavior` from Rust.** The binding may be missing from the AppKit crate in use; fall back to a raw Objective-C message send, guarded by an availability check for macOS 14.
4. **Built-in competition.** macOS 26 and later include a basic clipboard history in Spotlight. The differentiators here are the snippet library, longer retention, filters, and the CLI.
5. **Source-app attribution** is a heuristic (section 5.4). Acceptable, but the ignore list should be documented as best-effort.
6. **Rich text fidelity.** Is storing RTF/HTML worth the size? Proposed: store it, cap at 256 KB, measure.
7. **Should history be encrypted?** Deferred. If yes, SQLCipher with a Keychain-held key.
8. **Snippet sync.** `snippets.db` is isolated so it can later be synced or replaced by a remote SQLite service without touching history.

## 12. Milestones

| # | Scope | Done when |
|---|---|---|
| M0 | Feasibility spike (details below) | The M0 questions are answered in writing and the two demos work |
| M1 | Text capture, store, filters, retention, CLI `list`/`search` | Copies show up in `clipstash list`; ignored and concealed copies do not |
| M2 | Viewer: search, paste, copy back, delete, clear | Daily-drivable for text |
| M3 | Images and file clips, rich-text payload | All three clip types round-trip |
| M4 | Snippets: promote flow, library window, placeholders, import/export | A clip can be promoted, edited, and pasted with placeholders resolved |
| M5 | Merge, settings UI, full onboarding, pause, launch at login, notarized build | Feature parity with section 3 |
| v2 | Type-to-expand keywords | — |

M1 includes the tray icon, the access gate, and a minimal version of the clipboard-access onboarding step, since capture cannot be tested without them on enforcing systems.

### M0 detail

A tiny signed Tauri app with a tray icon, run on macOS 14, 15.4+ and 27.2+.

Questions to answer:

| # | Question |
|---|---|
| 1 | On 27.2, what is `accessBehavior` for a fresh app, and does a background read raise the alert by default? |
| 2 | Do `changeCount`, `types` and the `detect` methods work, without an alert, in each of the four states? |
| 3 | What does a content read return under deny, and how long does it take? |
| 4 | After the user sets Allow in Settings, does capture resume without relaunching? |
| 5 | Which Settings deep link opens the pasteboard pane on each OS version? |
| 6 | Do grants survive a rebuild and a version bump with the same signing certificate? |

Demos:

1. Hotkey opens a non-activating panel from an Accessory-policy app; Return pastes a hardcoded string into TextEdit, a browser field and a terminal.
2. With Accessibility withheld, the same flow ends in the copy-only fallback.

## 13. Testing

- `core/` unit tests: filters, dedupe, retention boundaries, placeholder expansion, FTS ranking.
- Property test: any sequence of capture, promote, clear and prune leaves snippets intact.
- Integration tests on macOS: write to the pasteboard from a helper process, assert rows; assert that concealed-type writes produce no row.
- Manual matrix for paste: native apps, Electron apps, terminal, browser text fields, secure-input fields.
- Permission matrix, per OS version (14, 15.4+, 27.2+): each pasteboard state × Accessibility on/off. Use `tccutil reset Pasteboard` and `tccutil reset Accessibility` on the development bundle id between runs. Assert the tray status, that no alert appears outside onboarding, and that capture resumes after a grant.

## 14. References

- Michael Tsai, "Pasteboard Privacy Preview in macOS 15.4": https://mjtsai.com/blog/2025/05/12/pasteboard-privacy-preview-in-macos-15-4/
- macOS 27 release notes summary (27.2 flag removal): https://blakecrosley.com/blog/macos-27-release-notes
- Access behavior states and exemptions: https://clipboardai.app/blog/articles/mac-paste-permission-prompt
- Grant persistence test on 27.0.1: https://github.com/code-devanshu/kopi/issues/21
- App Store rejection for `CGEvent.post`: https://developer.apple.com/forums/thread/820594
- Accessibility vs Input Monitoring: https://developer.apple.com/forums/thread/828052
- Accessibility pane rename in macOS 27: https://support.fiplab.com/copyclip-2/direct-pasting
- Tauri menu-bar pattern with `tauri-nspanel`: https://blog-ximei-me.pages.dev/blog/en/2025/0917-tauri-menubar-app
- Alfred Clipboard History (reference feature): https://www.alfredapp.com/help/features/clipboard/
