# clipstash

A macOS menu-bar clipboard history with a snippet library. Design: [clipboard-manager-design.md](clipboard-manager-design.md).

## Layout

| Path | What |
|---|---|
| `core/` | Store (SQLite + FTS5), search, capture filters, retention, placeholders. No OS dependencies. |
| `platform-macos/` | Pasteboard, access gate, frontmost app, ⌘V synthesis, merge event tap, launch at login. |
| `app/` | Tauri 2 shell: tray, watcher, paste engine, commands, viewer panel (`tauri-nspanel`). |
| `cli/` | `clipstash` binary over the same SQLite files. |
| `ui/` | TypeScript front end: viewer, snippet library, settings and onboarding. |

## Build and run

```sh
(cd ui && npm install)

# Development build, separate bundle id (com.clipstash.app.dev)
(cd app && cargo tauri build --debug --config tauri.dev.conf.json --bundles app)
open target/debug/bundle/macos/clipstash-dev.app

# Release build
(cd app && cargo tauri build)

# CLI
cargo install --path cli
```

Capture is off until you finish onboarding. Default hotkey: ⌥⌘C.

To work on the UI without the app, run `npm run dev` in `ui/` and open
`http://localhost:5173/viewer.html` (or `library.html`, `settings.html`): a mock backend is used
outside Tauri. Query flags: `?state=blocked`, `?access=default`, `?ax=0`.

## CLI

```
clipstash list [--kind text|image|file] [--limit 20] [--json]
clipstash search <query> [--filter all|text|image|file|snippet]
clipstash show <id> | copy <id> [--plain]
clipstash clear [5m|15m|all]        clipstash prune
clipstash snippet list|add|copy|export|import
clipstash library | viewer          # opens the window in the running app
clipstash doctor [--read]           # pasteboard access state and permissions
```

`CLIPSTASH_DIR` (or `--dir`) overrides the data directory, `~/Library/Application Support/clipstash`.

## Tests

```sh
cargo test --workspace            # unit, store and property tests
scripts/integration-test.sh       # runs the dev app; overwrites your clipboard
```

## Signing

Builds are ad-hoc signed (`signingIdentity: "-"`). Privacy grants are tied to the code signature, so
an ad-hoc build can lose its Accessibility and pasteboard grants on rebuild. For a build that keeps
them, and for distribution, set a Developer ID identity in `app/tauri.conf.json` and the notarization
environment variables Tauri reads (`APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`).

## Deviations from the design doc

- Settings live in a `meta` table in `snippets.db`.
- The rich payload's format (RTF or HTML) is sniffed from the bytes; the `rich` column is as specified.
- Classification: when plain text and RTF are both present next to an image, the clip is text
  (word processors and spreadsheets attach a rendered picture of copied text).
- `accessBehavior` is read on every poll tick, not only on change, so the tray and onboarding react
  as soon as access is granted.
- Image snippets export with extra `kind` and base64 `blob` fields.
