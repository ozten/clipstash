type Child = Node | string | null | undefined | false;
type Props = Record<string, unknown>;

/** Tiny DOM builder. Strings become text nodes, so content is never parsed as HTML. */
export function h<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  props: Props | null = null,
  ...children: (Child | Child[])[]
): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  for (const [key, value] of Object.entries(props ?? {})) {
    if (value === null || value === undefined || value === false) continue;
    if (key === "class") el.className = String(value);
    else if (key === "dataset") Object.assign(el.dataset, value);
    else if (key.startsWith("on") && typeof value === "function") {
      el.addEventListener(key.slice(2).toLowerCase(), value as EventListener);
    } else if (key in el) (el as unknown as Props)[key] = value;
    else el.setAttribute(key, String(value));
  }
  for (const child of children.flat()) {
    if (child === null || child === undefined || child === false) continue;
    el.append(child);
  }
  return el;
}

/** Trusted, constant SVG markup only. */
export function icon(name: keyof typeof ICONS): HTMLElement {
  const span = document.createElement("span");
  span.className = "icon";
  span.innerHTML = ICONS[name];
  return span;
}

const svg = (body: string) =>
  `<svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${body}</svg>`;

const ICONS = {
  text: svg('<path d="M3 4h10M3 8h10M3 12h6"/>'),
  link: svg('<path d="M6.5 9.5a3 3 0 0 0 4.2 0l2-2a3 3 0 0 0-4.2-4.2l-.8.8"/><path d="M9.5 6.5a3 3 0 0 0-4.2 0l-2 2a3 3 0 0 0 4.2 4.2l.8-.8"/>'),
  image: svg('<rect x="2" y="3" width="12" height="10" rx="1.5"/><circle cx="5.6" cy="6.4" r="1"/><path d="m3 12 3.5-3.5 2.5 2.5 2-2 2.5 2.5"/>'),
  file: svg('<path d="M4 2h5l3 3v9H4z"/><path d="M9 2v3h3"/>'),
  star: svg('<path d="m8 2 1.8 3.8 4.2.5-3.1 2.9.8 4.1L8 11.3l-3.7 2 .8-4.1L2 6.3l4.2-.5z" fill="currentColor" stroke="none"/>'),
  search: svg('<circle cx="7" cy="7" r="4.2"/><path d="m10.2 10.2 3.3 3.3"/>'),
  plus: svg('<path d="M8 3v10M3 8h10"/>'),
  trash: svg('<path d="M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5l.6 8.5h5.8l.6-8.5"/>'),
  check: svg('<path d="m3 8.5 3.2 3.2L13 4.8"/>'),
  warn: svg('<path d="M8 2.2 14.3 13H1.7z"/><path d="M8 6.5v3M8 11.4v.1"/>'),
};

export function relTime(ms: number, now = Date.now()): string {
  const s = Math.max(0, Math.round((now - ms) / 1000));
  if (s < 45) return "just now";
  if (s < 3600) return `${Math.max(1, Math.round(s / 60))} min ago`;
  if (s < 86_400) return `${Math.round(s / 3600)} h ago`;
  if (s < 2 * 86_400) return "yesterday";
  if (s < 30 * 86_400) return `${Math.round(s / 86_400)} days ago`;
  return new Date(ms).toLocaleDateString();
}

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(n < 10_240 ? 1 : 0)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

export function oneLine(s: string, max = 200): string {
  const flat = s.replace(/\s+/g, " ").trim();
  return flat.length > max ? flat.slice(0, max) + "…" : flat;
}

export function debounce<A extends unknown[]>(fn: (...args: A) => void, ms: number): (...args: A) => void {
  let timer: number | undefined;
  return (...args: A) => {
    window.clearTimeout(timer);
    timer = window.setTimeout(() => fn(...args), ms);
  };
}

export function errorText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
}

const MOD_SYMBOLS: Record<string, string> = { Control: "⌃", Alt: "⌥", Shift: "⇧", Super: "⌘" };
const KEY_NAMES: Record<string, string> = {
  Space: "Space", Enter: "↩", Tab: "⇥", Backspace: "⌫", Delete: "⌦", Escape: "⎋",
  ArrowUp: "↑", ArrowDown: "↓", ArrowLeft: "←", ArrowRight: "→",
  Minus: "-", Equal: "=", Comma: ",", Period: ".", Slash: "/", Backslash: "\\",
  Semicolon: ";", Quote: "'", BracketLeft: "[", BracketRight: "]", Backquote: "`",
};

/** "Alt+Super+KeyC" → "⌥⌘C". */
export function prettyHotkey(hotkey: string): string {
  return hotkey
    .split("+")
    .map((part) => MOD_SYMBOLS[part] ?? KEY_NAMES[part] ?? part.replace(/^Key|^Digit/, ""))
    .join("");
}

/** Build a hotkey string from a key event, or null if it is not a usable combination yet. */
export function hotkeyFromEvent(e: KeyboardEvent): string | null {
  if (["Meta", "Control", "Alt", "Shift"].includes(e.key)) return null;
  const mods = [e.ctrlKey && "Control", e.altKey && "Alt", e.shiftKey && "Shift", e.metaKey && "Super"].filter(
    Boolean,
  ) as string[];
  const isFunctionKey = /^F\d+$/.test(e.code);
  // A global hotkey without ⌘, ⌃ or ⌥ would swallow ordinary typing.
  if (!isFunctionKey && !(e.metaKey || e.ctrlKey || e.altKey)) return null;
  return [...mods, e.code].join("+");
}
