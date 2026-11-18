# M0 findings

Measured on one machine only: macOS 26.6.2 (25G83), Apple silicon, ad-hoc signed build launched
from a terminal. macOS 14, 15.4–15.x and 27.2+ have **not** been tested; the questions that depend
on them are still open.

## Questions

| # | Question | Status |
|---|---|---|
| 1 | On 27.2, `accessBehavior` for a fresh app; does a background read alert by default? | Open. Needs a 27.2 machine. On 26.6.2 the state was `alwaysAllow` and no alert appeared, but the process was launched from a terminal, which may be what macOS attributed the access to. Not a clean fresh-app result. |
| 2 | Do `changeCount`, `types`, `detect` work without an alert in each state? | Partly. In `alwaysAllow`: `changeCount` ≈ 0.05 ms, `types` ≈ 1.5 ms, no alert. Other three states untested. `detect` methods are not used by the app. |
| 3 | What does a content read return under deny, and how long does it take? | Open. `clipstash doctor --read` times one read for this. |
| 4 | After Allow in Settings, does capture resume without relaunch? | Open. The watcher re-reads `accessBehavior` every tick, so it will resume if the property updates live. |
| 5 | Which Settings deep link opens the pasteboard pane? | Open. The app tries the `com.apple.settings.PrivacySecurity.extension` form, then the `com.apple.preference.security` form. |
| 6 | Do grants survive a rebuild and a version bump with the same certificate? | Open. No signing certificate on this machine; builds are ad-hoc. |

To collect the rest: on each OS version run `tccutil reset Pasteboard com.clipstash.app.dev`, launch
the dev app, and record `clipstash doctor --read` in each state.

## Demos

1. **Hotkey → non-activating panel → Return pastes.** Works in TextEdit on 26.6.2: with TextEdit
   frontmost, ⌥⌘C showed the panel (window level 4) while TextEdit stayed the frontmost app, and
   Return pasted the top clip into the document. A snippet with `{cursor}` and `{date}` pasted with
   the caret placed correctly. A browser field and a terminal were not tried.
2. **Copy-only fallback with Accessibility withheld.** Not exercised on the real app (Accessibility
   was already granted here). The code path returns "not pasted" and the viewer shows
   "Copied — press ⌘V"; that UI was checked against the mock backend only.
