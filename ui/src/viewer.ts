// Viewer panel (design section 7): filter by typing, paste with Return.
import "./mock";
import "./common.css";
import "./viewer.css";
import { api, on, type ClipDetail, type Collection, type Filter, type Hit, type Status, type Target } from "./api";
import { debounce, errorText, formatBytes, h, icon, oneLine, relTime } from "./util";

const LIMIT = 80;
const FILTERS: [Filter, string][] = [
  ["all", "All"],
  ["text", "Text"],
  ["image", "Images"],
  ["file", "Files"],
  ["snippet", "Snippets"],
];

let hits: Hit[] = [];
let selected = 0;
let filter: Filter = "all";
let searchSeq = 0;
let detailSeq = 0;
let sheetOpen = false;
let collections = new Map<number, string>();

const input = h("input", { type: "text", placeholder: "Filter clipboard history…", spellcheck: false, autocomplete: "off" });
const typeMenu = h(
  "select",
  { title: "Show (Tab)" },
  FILTERS.map(([value, label]) => h("option", { value }, label)),
);
const banner = h("div", { class: "banner", hidden: true });
const list = h("div", { class: "list" });
const preview = h("div", { class: "preview" });
const footer = h("div", { class: "footer" });
const overlay = h("div");
const panel = h(
  "div",
  { class: "panel" },
  h("div", { class: "searchbar" }, icon("search"), input, typeMenu),
  banner,
  h("div", { class: "body" }, list, preview),
  footer,
  overlay,
);
document.getElementById("app")!.append(panel);

function targetOf(hit: Hit): Target {
  return hit.type === "clip" ? { clip: hit.item.id } : { snippet: hit.item.id };
}

function current(): Hit | undefined {
  return hits[selected];
}

// ---- list -----------------------------------------------------------------

function rowFor(hit: Hit, index: number): HTMLElement {
  const hint = index < 9 ? `⌘${index + 1}` : "";
  let lead: HTMLElement;
  let label: string;
  let tag = "";
  if (hit.type === "snippet") {
    lead = icon("star");
    label = hit.item.name;
    tag = collections.get(hit.item.collectionId) ?? "Snippets";
  } else if (hit.item.kind === "image") {
    lead = h("img", { class: "thumb", src: `stash://localhost/clip/${hit.item.id}`, alt: "" });
    label = hit.item.preview;
  } else {
    lead = icon(hit.item.kind === "file" ? "file" : hit.item.isUrl ? "link" : "text");
    label = oneLine(hit.item.preview, 160);
  }
  return h(
    "div",
    {
      class: `row ${hit.type}${index === selected ? " selected" : ""}`,
      onmousedown: (e: MouseEvent) => {
        e.preventDefault();
        select(index);
      },
      ondblclick: () => void paste(index, false),
    },
    lead,
    h("span", { class: "label" }, label),
    tag && h("span", { class: "tag" }, tag),
    h("span", { class: "hint" }, hint),
  );
}

function renderList() {
  if (hits.length === 0) {
    const query = input.value.trim();
    list.replaceChildren(
      h(
        "div",
        { class: "empty" },
        h("strong", null, query ? "No matches" : filter === "snippet" ? "No snippets yet" : "Nothing copied yet"),
        h("span", null, query ? "Try a shorter filter, or a different type." : filter === "snippet" ? "Press ⌘S on a clip to save it as a snippet." : "Copy something and it will show up here."),
      ),
    );
    preview.replaceChildren();
    return;
  }
  list.replaceChildren(...hits.map(rowFor));
  renderPreview();
}

function select(index: number) {
  if (hits.length === 0) return;
  selected = Math.max(0, Math.min(hits.length - 1, index));
  [...list.children].forEach((el, i) => el.classList.toggle("selected", i === selected));
  list.children[selected]?.scrollIntoView({ block: "nearest" });
  renderPreview();
}

// ---- preview --------------------------------------------------------------

function meta(...parts: (string | null | undefined | false)[]): HTMLElement {
  return h("div", { class: "meta" }, parts.filter(Boolean).map((p) => h("span", null, p as string)));
}

async function renderPreview() {
  const hit = current();
  const seq = ++detailSeq;
  if (!hit) {
    preview.replaceChildren();
    return;
  }
  if (hit.type === "snippet") {
    const s = hit.item;
    const body = s.kind === "image"
      ? h("div", { class: "content image" }, h("img", { src: `stash://localhost/snippet/${s.id}?v=${s.updatedAt}`, alt: "" }))
      : h("div", { class: "content" }, h("span", { class: "title" }, s.name), s.body);
    preview.replaceChildren(
      body,
      meta(collections.get(s.collectionId) ?? "Snippets", s.keyword, `Used ${s.useCount}×`, s.originApp && `from ${s.originApp}`),
    );
    return;
  }
  let d: ClipDetail;
  try {
    d = await api.getClip(hit.item.id);
  } catch {
    return;
  }
  if (seq !== detailSeq) return;
  let content: HTMLElement;
  let size: string;
  if (d.kind === "image") {
    content = h("div", { class: "content image" }, h("img", { src: d.imageUrl, alt: "" }));
    size = `${(d.text ?? "").replace(/^\[image |\]$/g, "")} · ${formatBytes(d.byteSize)}`;
  } else if (d.kind === "file") {
    const paths = d.paths ?? [];
    content = h("div", { class: "content" }, paths.map((p) => h("div", { class: "path" }, icon("file"), p)));
    size = paths.length === 1 ? "1 file" : `${paths.length} files`;
  } else {
    content = h("div", { class: "content" }, d.text ?? "");
    const chars = d.charCount ?? 0;
    size = `${chars.toLocaleString()} char${chars === 1 ? "" : "s"}`;
  }
  preview.replaceChildren(content, meta(d.appName, relTime(d.lastUsed), size, d.hasRich && "formatted"));
}

// ---- data -----------------------------------------------------------------

async function refresh(keepSelection = false) {
  const seq = ++searchSeq;
  const keep = keepSelection ? current() : undefined;
  let result: Hit[];
  try {
    result = await api.search(input.value, filter, LIMIT);
  } catch (e) {
    console.error(e);
    return;
  }
  if (seq !== searchSeq) return;
  hits = result;
  selected = 0;
  if (keep) {
    const i = hits.findIndex((x) => x.type === keep.type && x.item.id === keep.item.id);
    if (i >= 0) selected = i;
  }
  renderList();
  list.children[selected]?.scrollIntoView({ block: "nearest" });
}

async function loadCollections() {
  try {
    const all: Collection[] = await api.listCollections();
    collections = new Map(all.map((c) => [c.id, c.name]));
  } catch {
    /* names are decorative */
  }
}

async function renderBanner() {
  let status: Status;
  try {
    status = await api.getStatus();
  } catch {
    return;
  }
  const messages: Partial<Record<Status["state"], [string, string, () => void]>> = {
    off: ["Capture is off. Nothing is being recorded.", "Set up…", () => void api.openSettings(true)],
    blocked: ["Clipboard access is needed to record new clips.", "Fix…", () => void api.openSettings(true)],
    paused: ["Capture is paused.", "Resume", () => void api.setPaused("resume")],
  };
  const m = messages[status.state];
  banner.hidden = !m;
  if (m) banner.replaceChildren(icon("warn"), h("span", null, m[0]), h("span", { class: "spacer" }), h("button", { onclick: m[2] }, m[1]));
}

function renderFooter() {
  const hint = (keys: string, label: string) => h("span", null, h("kbd", null, keys), label);
  footer.replaceChildren(
    hint("↩", "Paste"),
    hint("⇧↩", "Plain"),
    hint("⌘C", "Copy"),
    hint("⌘S", "Save snippet"),
    hint("fn⌫", "Delete"),
    h("span", { class: "spacer" }),
    hint("⌘L", "Library"),
    hint("esc", "Close"),
  );
}

// ---- actions --------------------------------------------------------------

function toast(message: string, ms: number): Promise<void> {
  const el = h("div", { class: "toast" }, message);
  overlay.append(el);
  return new Promise((resolve) => window.setTimeout(() => (el.remove(), resolve()), ms));
}

async function paste(index: number, plain: boolean) {
  const hit = hits[index];
  if (!hit) return;
  try {
    const pasted = await api.paste(targetOf(hit), plain);
    if (!pasted) {
      // Copy-only fallback: Accessibility is not granted (or secure input is on).
      await toast("Copied — press ⌘V", 900);
      await api.hideViewer();
    }
  } catch (e) {
    await toast(errorText(e), 1600);
  }
}

async function copySelected() {
  const hit = current();
  if (!hit) return;
  try {
    await api.copy(targetOf(hit));
    await toast("Copied", 450);
    await api.hideViewer();
  } catch (e) {
    await toast(errorText(e), 1600);
  }
}

async function deleteSelected() {
  const hit = current();
  if (!hit || hit.type !== "clip") return;
  const index = selected;
  await api.deleteClip(hit.item.id);
  hits.splice(index, 1);
  selected = Math.min(index, hits.length - 1);
  renderList();
}

function setFilter(next: Filter) {
  filter = next;
  typeMenu.value = next;
  void refresh();
}

// ---- promote sheet (design section 8.1) -------------------------------------

async function openPromote() {
  const hit = current();
  if (!hit || hit.type !== "clip" || sheetOpen) return;
  const clipId = hit.item.id;
  let defaults;
  let cols: Collection[];
  try {
    [defaults, cols] = await Promise.all([api.promoteDefaults(clipId), api.listCollections()]);
  } catch (e) {
    await toast(errorText(e), 1600);
    return;
  }
  sheetOpen = true;
  const NEW = "__new__";
  const name = h("input", { type: "text", value: defaults.name });
  const collection = h(
    "select",
    null,
    cols.map((c) => h("option", { value: String(c.id), selected: c.id === defaults.collectionId }, c.name)),
    h("option", { value: NEW }, "New collection…"),
  );
  const newName = h("input", { type: "text", placeholder: "Collection name", hidden: true });
  collection.addEventListener("change", () => {
    newName.hidden = collection.value !== NEW;
    if (!newName.hidden) newName.focus();
  });
  const keyword = h("input", { type: "text", placeholder: "Optional, e.g. ;sig", spellcheck: false });
  const body = h("textarea", { spellcheck: false });
  body.value = defaults.body;
  const error = h("span", { class: "error" });

  const close = () => {
    sheetOpen = false;
    backdrop.remove();
    input.focus();
  };
  const save = async () => {
    try {
      const collectionId =
        collection.value === NEW ? (await api.createCollection(newName.value)).id : Number(collection.value);
      await api.promote(clipId, {
        collectionId,
        name: name.value,
        keyword: keyword.value.trim() || null,
        body: defaults.imageUrl ? defaults.body : body.value,
      });
      close();
      await loadCollections();
      void toast("Saved to snippets", 700);
    } catch (e) {
      error.textContent = errorText(e);
    }
  };

  const sheet = h(
    "div",
    { class: "sheet" },
    h("h2", null, "Save as snippet"),
    h("label", null, h("span", null, "Name"), name),
    h("label", null, h("span", null, "Collection"), h("div", { style: "display:grid;gap:6px" }, collection, newName)),
    h("label", null, h("span", null, "Keyword"), keyword),
    defaults.imageUrl
      ? h("label", { class: "top" }, h("span", null, "Image"), h("img", { src: defaults.imageUrl, alt: "" }))
      : h("label", { class: "top" }, h("span", null, "Body"), body),
    h(
      "div",
      { class: "actions" },
      error,
      h("button", { onclick: close }, "Cancel"),
      h("button", { class: "primary", onclick: () => void save() }, "Save ↩"),
    ),
  );
  sheet.addEventListener("keydown", (e) => {
    e.stopPropagation();
    if (e.key === "Escape") {
      e.preventDefault();
      close();
    } else if (e.key === "Enter" && (e.target !== body || e.metaKey)) {
      // Return saves; in the body it inserts a newline unless ⌘ is held.
      e.preventDefault();
      void save();
    }
  });
  const backdrop = h("div", { class: "sheet-backdrop", onmousedown: (e: MouseEvent) => e.target === backdrop && close() }, sheet);
  overlay.append(backdrop);
  name.focus();
  name.select();
}

// ---- keys (design section 7.3) ----------------------------------------------

window.addEventListener("keydown", (e) => {
  if (sheetOpen) return;
  const cmd = e.metaKey && !e.ctrlKey && !e.altKey;
  const done = () => e.preventDefault();

  if (e.key === "ArrowDown" || (e.ctrlKey && e.key === "n")) return done(), select(selected + 1);
  if (e.key === "ArrowUp" || (e.ctrlKey && e.key === "p")) return done(), select(selected - 1);
  if (e.key === "PageDown") return done(), select(selected + 8);
  if (e.key === "PageUp") return done(), select(selected - 8);
  if (e.key === "Enter") return done(), void paste(selected, e.shiftKey);
  if (e.key === "Escape") return done(), void api.hideViewer();
  if (e.key === "Tab") {
    done();
    const i = FILTERS.findIndex(([f]) => f === filter);
    return setFilter(FILTERS[(i + (e.shiftKey ? FILTERS.length - 1 : 1)) % FILTERS.length][0]);
  }
  // Fn+Delete is the forward-delete key; ⌘⌫ is offered as an alias.
  if (e.key === "Delete" || (cmd && e.key === "Backspace")) return done(), void deleteSelected();
  if (!cmd) return;
  if (e.code === "KeyC") return done(), void copySelected();
  if (e.code === "KeyS") return done(), void openPromote();
  if (e.code === "KeyL") return done(), void api.openLibrary();
  if (e.key === ",") return done(), void api.openSettings(false);
  if (/^Digit[1-9]$/.test(e.code)) return done(), void paste(Number(e.code.slice(5)) - 1, e.shiftKey);
});

input.addEventListener("input", debounce(() => void refresh(), 30));
typeMenu.addEventListener("change", () => {
  setFilter(typeMenu.value as Filter);
  input.focus();
});

// ---- lifecycle ------------------------------------------------------------

async function shown() {
  overlay.replaceChildren();
  sheetOpen = false;
  input.value = "";
  filter = "all";
  typeMenu.value = "all";
  input.focus();
  await Promise.all([loadCollections(), renderBanner()]);
  await refresh();
}

void on("viewer-shown", () => void shown());
void on("clip-added", () => void refresh(true));
void on("clip-removed", () => void refresh(true));
void on("snippets-changed", () => void loadCollections().then(() => refresh(true)));
void on("status-changed", () => void renderBanner());

renderFooter();
void shown();
