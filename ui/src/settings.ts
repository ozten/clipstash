// Settings window, and the onboarding flow (design section 10.3).
import "./mock";
import "./common.css";
import "./settings.css";
import { ask } from "@tauri-apps/plugin-dialog";
import { api, on, type Retention, type Settings, type Status } from "./api";
import { errorText, h, hotkeyFromEvent, icon, prettyHotkey } from "./util";

const root = document.getElementById("app")!;
let settings: Settings;
let status: Status;
let mode: "onboarding" | "settings" = "settings";
let step = 0;
let notice = "";

async function load() {
  [settings, status] = await Promise.all([api.getSettings(), api.getStatus()]);
}

async function update(patch: Partial<Settings>) {
  notice = "";
  try {
    settings = await api.setSettings({ ...settings, ...patch });
  } catch (e) {
    // The backend may save and still report a problem (for example the
    // merge tap could not be installed), so re-read what is in effect.
    notice = errorText(e);
    settings = await api.getSettings();
  }
  status = await api.getStatus();
  render();
}

// ---- shared controls --------------------------------------------------------

function toggle(key: keyof Settings): HTMLInputElement {
  return h("input", {
    type: "checkbox",
    class: "switch",
    checked: Boolean(settings[key]),
    onchange: (e: Event) => void update({ [key]: (e.target as HTMLInputElement).checked } as Partial<Settings>),
  });
}

function line(title: string, detail: string | null, ...controls: (Node | null | false)[]): HTMLElement {
  return h("div", { class: "line" }, h("div", { class: "text" }, title, detail && h("small", null, detail)), ...controls);
}

let recording = false;

function hotkeyButton(): HTMLButtonElement {
  const button = h("button", { class: "hotkey" }, prettyHotkey(settings.hotkey));
  const stop = () => {
    recording = false;
    button.classList.remove("recording");
    button.textContent = prettyHotkey(settings.hotkey);
    window.removeEventListener("keydown", onKey, true);
  };
  const onKey = (e: KeyboardEvent) => {
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape") return stop();
    const combo = hotkeyFromEvent(e);
    if (!combo) return;
    stop();
    void update({ hotkey: combo });
  };
  button.addEventListener("click", () => {
    if (recording) return stop();
    recording = true;
    button.classList.add("recording");
    button.textContent = "Type shortcut…";
    window.addEventListener("keydown", onKey, true);
  });
  button.addEventListener("blur", () => recording && stop());
  return button;
}

/** True when background capture is allowed, or there is nothing to grant. */
function clipboardOk(): boolean {
  return status.accessBehavior === "alwaysAllow" || status.accessBehavior === "unavailable";
}

function clipboardLabel(): [string, string] {
  switch (status.accessBehavior) {
    case "unavailable":
      return ["ok", "No permission needed"];
    case "alwaysAllow":
      return ["ok", "Allowed"];
    case "alwaysDeny":
      return ["warn", "Denied"];
    default:
      return ["warn", "Not granted"];
  }
}

// ---- onboarding -------------------------------------------------------------

function onboarding(): HTMLElement {
  const total = 4;
  const next = () => {
    step++;
    render();
  };
  const finish = async () => {
    await update({ captureEnabled: true, onboardingDone: true });
    mode = "settings";
    render();
  };
  const back = step > 0 && h("button", { class: "ghost", onclick: () => (step--, render()) }, "Back");
  let content: (Node | false | null)[];

  if (step === 0) {
    const item = (title: string, detail: string) => h("li", null, icon("check"), h("div", null, title, h("small", null, detail)));
    content = [
      h("h1", null, "Welcome to clipstash"),
      h("p", null, "A clipboard history you can search, and a snippet library for the things you paste again and again."),
      h(
        "ul",
        null,
        item("What is captured", "Text, links, images and file references that you copy, with the name of the app they came from."),
        item("What is skipped", "Anything marked as concealed by its source, such as passwords, and everything copied from apps on your ignore list. Password managers are on it already."),
        item("Where it goes", "Two SQLite files on this Mac. clipstash makes no network requests, and history expires on a schedule you choose."),
      ),
      h("div", { class: "actions" }, h("span", { class: "spacer" }), h("button", { class: "primary", onclick: next }, "Continue")),
    ];
  } else if (step === 1) {
    const ok = clipboardOk();
    const [kind, label] = clipboardLabel();
    content = [
      h("h1", null, "Clipboard access"),
      ok
        ? h(
            "p",
            null,
            status.accessBehavior === "unavailable"
              ? `macOS ${status.osVersion} does not ask for permission to read the clipboard, so there is nothing to grant.`
              : "clipstash is allowed to read the clipboard in the background.",
          )
        : h(
            "p",
            null,
            "macOS asks before an app reads the clipboard in the background. Choose Request Access: macOS shows its alert once, then clipstash appears in System Settings under Privacy & Security › Paste from Other Apps. Set it to Allow there.",
          ),
      h("div", { class: "state" }, h("span", { class: `pill ${kind}` }, label)),
      !ok &&
        h(
          "div",
          { class: "mock" },
          h("div", { class: "mhead" }, "Privacy & Security › Paste from Other Apps"),
          h("div", { class: "mrow" }, h("span", { class: "app" }), "clipstash", h("span", { class: "sel" }, "Allow ⌃")),
        ),
      !ok && h("p", null, "This screen moves on by itself once access is allowed."),
      h(
        "div",
        { class: "actions" },
        back,
        h("span", { class: "spacer" }),
        !ok && h("button", { class: "ghost", onclick: next }, "Not Now"),
        ok
          ? h("button", { class: "primary", onclick: next }, "Continue")
          : h("button", { class: "primary", onclick: () => void api.requestClipboardAccess().then(refreshStatus) }, "Request Access"),
      ),
    ];
  } else if (step === 2) {
    const pane = status.accessibilityPaneName;
    content = [
      h("h1", null, "Paste directly (optional)"),
      h(
        "p",
        null,
        `With permission, pressing Return in the viewer pastes straight into the app you were using. macOS lists this under Privacy & Security › ${pane}.`,
      ),
      h("p", null, "Without it, clipstash copies the clip and you press ⌘V yourself. Everything else works the same."),
      h(
        "div",
        { class: "state" },
        h("span", { class: `pill ${status.accessibility ? "ok" : "off"}` }, status.accessibility ? "Granted" : "Not granted"),
      ),
      h(
        "div",
        { class: "actions" },
        back,
        h("span", { class: "spacer" }),
        !status.accessibility && h("button", { class: "ghost", onclick: next }, "Skip, I’ll paste with ⌘V"),
        status.accessibility
          ? h("button", { class: "primary", onclick: next }, "Continue")
          : h("button", { class: "primary", onclick: () => void api.requestAccessibility().then(refreshStatus) }, `Open ${pane}`),
      ),
    ];
  } else {
    content = [
      h("h1", null, "Your shortcut"),
      h("p", null, "Press this anywhere to open your clipboard history. Type to filter, Return to paste, ⌘S to keep a clip as a snippet."),
      h("div", { class: "card" }, line("Open clipboard history", "Click to change", hotkeyButton())),
      notice !== "" && h("div", { class: "notice error" }, notice),
      h("p", null, "Finishing turns capture on. You can pause it at any time from the menu-bar icon."),
      h(
        "div",
        { class: "actions" },
        back,
        h("span", { class: "spacer" }),
        h("button", { class: "primary", onclick: () => void finish() }, "Start Capturing"),
      ),
    ];
  }

  return h(
    "div",
    { class: "onb" },
    h("div", { class: "steps" }, Array.from({ length: total }, (_, i) => h("i", { class: i <= step ? "done" : "" }))),
    ...content,
  );
}

// ---- settings ---------------------------------------------------------------

const RETENTION: [Retention, string][] = [
  ["24h", "24 hours"],
  ["7d", "7 days"],
  ["1m", "1 month"],
  ["3m", "3 months"],
];

function retention(key: "retentionText" | "retentionImages" | "retentionFiles"): HTMLSelectElement {
  return h(
    "select",
    { title: "Keep for", onchange: (e: Event) => void update({ [key]: (e.target as HTMLSelectElement).value as Retention }) },
    RETENTION.map(([value, label]) => h("option", { value, selected: settings[key] === value }, label)),
  );
}

function sizeInput(key: "maxTextBytes" | "maxImageBytes", unit: number): HTMLInputElement {
  return h("input", {
    type: "number",
    min: "1",
    value: String(Math.max(1, Math.round(settings[key] / unit))),
    onchange: (e: Event) => {
      const n = Number((e.target as HTMLInputElement).value);
      if (Number.isFinite(n) && n >= 1) void update({ [key]: Math.round(n * unit) });
      else render();
    },
  });
}

function stateLine(): HTMLElement {
  const labels: Record<Status["state"], [string, string, string]> = {
    capturing: ["ok", "Capturing", "New copies are being recorded."],
    paused: ["warn", "Paused", "Copies are not recorded until you resume."],
    blocked: ["warn", "Clipboard access needed", "macOS is not letting clipstash read the clipboard in the background."],
    off: ["off", "Off", "Nothing is recorded until you turn capture on."],
  };
  const [kind, label, detail] = labels[status.state];
  return line(
    "Capture clipboard history",
    detail,
    h("span", { class: `pill ${kind}` }, label),
    status.state === "paused" && h("button", { onclick: () => void api.setPaused("resume").then(refreshStatus) }, "Resume"),
    status.state === "blocked" && h("button", { onclick: startOnboarding }, "Fix…"),
    toggle("captureEnabled"),
  );
}

function settingsView(): HTMLElement {
  const ignore = h("textarea", { spellcheck: false, placeholder: "One bundle id per line, e.g. com.example.app" });
  ignore.value = settings.ignoreApps.join("\n");
  ignore.addEventListener("change", () => void update({ ignoreApps: ignore.value.split("\n") }));
  const [clipKind, clipLabel] = clipboardLabel();

  const clear = async (scope: "5m" | "15m" | "all", label: string) => {
    if (scope === "all" && !(await ask("Delete all clipboard history? Snippets are not affected.", { title: "Clear history", kind: "warning" }))) return;
    const n = await api.clearHistory(scope);
    notice = "";
    await refreshStatus();
    clearInfo = `Removed ${n} clip${n === 1 ? "" : "s"} (${label}).`;
    render();
  };

  return h(
    "div",
    { class: "wrap" },
    h("h1", null, "clipstash"),
    notice && h("div", { class: "notice error" }, notice),

    h(
      "section",
      null,
      h("h2", null, "General"),
      h(
        "div",
        { class: "card" },
        stateLine(),
        line("Open clipboard history", "Global shortcut. Click to change.", hotkeyButton()),
        line("Launch at login", null, toggle("launchAtLogin")),
      ),
    ),

    h(
      "section",
      null,
      h("h2", null, "What to keep"),
      h(
        "div",
        { class: "card" },
        line("Text and links", `Skip text over ${Math.round(settings.maxTextBytes / 1024)} KB`, sizeInput("maxTextBytes", 1024), h("span", { class: "dim" }, "KB"), retention("retentionText"), toggle("captureText")),
        line("Images", `Skip images over ${Math.round(settings.maxImageBytes / 1024 / 1024)} MB`, sizeInput("maxImageBytes", 1024 * 1024), h("span", { class: "dim" }, "MB"), retention("retentionImages"), toggle("captureImages")),
        line("Files", "Stores the paths, not the files themselves", retention("retentionFiles"), toggle("captureFiles")),
      ),
    ),

    h(
      "section",
      null,
      h("h2", null, "Ignored apps"),
      h(
        "div",
        { class: "card" },
        h(
          "div",
          { class: "line stack" },
          h("div", { class: "text" }, "Never record copies made while these apps are in front", h("small", null, "Best effort: macOS does not say which app wrote to the clipboard, so clipstash goes by the frontmost app. Content that apps mark as concealed is always skipped.")),
          ignore,
          h(
            "div",
            { class: "rowend" },
            h("button", { onclick: () => void api.defaultSettings().then((d) => update({ ignoreApps: d.ignoreApps })) }, "Restore Defaults"),
          ),
        ),
      ),
    ),

    h(
      "section",
      null,
      h("h2", null, "Merge"),
      h(
        "div",
        { class: "card" },
        line(
          "Append with ⌘C C",
          settings.mergeEnabled && !status.mergeTapActive
            ? `Not active: needs ${status.accessibilityPaneName} or Input Monitoring permission.`
            : "Hold ⌘ and tap C twice to add the selection to the previous clip.",
          settings.mergeEnabled && !status.mergeTapActive && h("button", { onclick: () => void api.openPrivacyPane("inputMonitoring") }, "Open Settings"),
          toggle("mergeEnabled"),
        ),
        line(
          "Join with",
          null,
          h(
            "select",
            { disabled: !settings.mergeEnabled, onchange: (e: Event) => void update({ mergeSeparator: (e.target as HTMLSelectElement).value as Settings["mergeSeparator"] }) },
            (["newline", "space", "none"] as const).map((v) => h("option", { value: v, selected: settings.mergeSeparator === v }, { newline: "New line", space: "Space", none: "Nothing" }[v])),
          ),
        ),
        line("Play a sound when merged", null, toggle("sound")),
      ),
    ),

    h(
      "section",
      null,
      h("h2", null, "Permissions"),
      h(
        "div",
        { class: "card" },
        line(
          "Clipboard access",
          status.accessBehavior === "unavailable" ? `Not required on macOS ${status.osVersion}` : "Privacy & Security › Paste from Other Apps",
          h("span", { class: `pill ${clipKind}` }, clipLabel),
          !clipboardOk() && h("button", { onclick: startOnboarding }, "Set Up…"),
        ),
        line(
          "Direct paste",
          status.accessibility ? `Privacy & Security › ${status.accessibilityPaneName}` : "Optional. Without it, clips are copied and you press ⌘V.",
          h("span", { class: `pill ${status.accessibility ? "ok" : "off"}` }, status.accessibility ? "Granted" : "Copy only"),
          !status.accessibility && h("button", { onclick: () => void api.requestAccessibility().then(refreshStatus) }, "Grant…"),
        ),
      ),
    ),

    h(
      "section",
      null,
      h("h2", null, "History"),
      h(
        "div",
        { class: "card" },
        line(
          `${status.clipCount.toLocaleString()} clip${status.clipCount === 1 ? "" : "s"} · ${status.snippetCount.toLocaleString()} snippet${status.snippetCount === 1 ? "" : "s"}`,
          clearInfo || "Clearing history never touches snippets.",
          h("button", { onclick: () => void clear("5m", "last 5 minutes") }, "Clear 5 min"),
          h("button", { onclick: () => void clear("15m", "last 15 minutes") }, "Clear 15 min"),
          h("button", { class: "danger", onclick: () => void clear("all", "all history") }, "Clear All"),
        ),
        line("Snippet library", null, h("button", { onclick: () => void api.openLibrary() }, "Open Library")),
        line("Setup guide", null, h("button", { onclick: startOnboarding }, "Show Again")),
      ),
    ),
  );
}

let clearInfo = "";

// ---- lifecycle --------------------------------------------------------------

function startOnboarding() {
  mode = "onboarding";
  step = 0;
  render();
}

function render() {
  if (recording) return;
  const scroll = document.documentElement.scrollTop;
  const active = document.activeElement;
  // Do not rebuild underneath a field the user is typing in.
  if (active instanceof HTMLTextAreaElement || (active instanceof HTMLInputElement && active.type === "number")) {
    if (root.contains(active) && mode === "settings") return;
  }
  root.replaceChildren(mode === "onboarding" ? onboarding() : settingsView());
  document.documentElement.scrollTop = scroll;
}

async function refreshStatus() {
  const before = status;
  status = await api.getStatus();
  // Onboarding advances by itself when a grant arrives.
  if (mode === "onboarding") {
    if (step === 1 && !(before.accessBehavior === "alwaysAllow") && status.accessBehavior === "alwaysAllow") step = 2;
    else if (step === 2 && !before.accessibility && status.accessibility) step = 3;
  }
  if (JSON.stringify(before) !== JSON.stringify(status)) render();
}

async function start() {
  await load();
  if (location.hash === "#onboarding" || !settings.onboardingDone) mode = "onboarding";
  render();
  void on("status-changed", () => void refreshStatus());
  void on("clip-added", () => void refreshStatus());
  void on("clip-removed", () => void refreshStatus());
  void on("snippets-changed", () => void refreshStatus());
  void on("show-onboarding", startOnboarding);
  // Accessibility has no change notification; poll while the window is open.
  window.setInterval(() => void refreshStatus(), 1500);
}

void start();
