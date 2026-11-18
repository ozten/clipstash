// Snippet library window (design section 8.2): collections, snippet list, editor.
import "./mock";
import "./common.css";
import "./library.css";
import { ask, message, open, save } from "@tauri-apps/plugin-dialog";
import { api, on, type Collection, type Snippet, type SnippetPatch } from "./api";
import { debounce, errorText, h, icon, oneLine } from "./util";

const PLACEHOLDERS = ["{date}", "{time}", "{clipboard}", "{cursor}"];
const DRAG_SNIPPET = "application/x-clipstash-snippet";
const DRAG_COLLECTION = "application/x-clipstash-collection";

let collections: Collection[] = [];
let snippets: Snippet[] = [];
/** null = "All". */
let currentCollection: number | null = null;
let currentSnippet: number | null = null;
let query = "";
/** Set while the editor has unsaved edits. */
let dirty = false;
let saving: Promise<void> = Promise.resolve();

const collectionList = h("div", { class: "items" });
const snippetList = h("div", { class: "items" });
const editorPane = h("div", { class: "pane" });
const filterInput = h("input", { type: "search", placeholder: "Filter snippets", spellcheck: false });

document.getElementById("app")!.append(
  h(
    "div",
    { class: "lib" },
    h(
      "div",
      { class: "pane" },
      h("header", null, "Collections", h("span", { class: "spacer" }), h("button", { class: "ghost", title: "New collection", onclick: () => void newCollection() }, icon("plus"))),
      collectionList,
      h(
        "footer",
        null,
        h("button", { onclick: () => void importCollection() }, "Import…"),
        h("button", { onclick: () => void exportCollection() }, "Export…"),
        h("button", { class: "danger", title: "Delete the selected collection", onclick: () => void deleteCurrentCollection() }, "Delete"),
      ),
    ),
    h(
      "div",
      { class: "pane" },
      h("header", null, "Snippets", h("span", { class: "spacer" }), h("button", { class: "ghost", title: "New snippet (⌘N)", onclick: () => void newSnippet() }, icon("plus"))),
      snippetList,
      h("footer", null, filterInput),
    ),
    editorPane,
  ),
);

// ---- collections ------------------------------------------------------------

function renderCollections() {
  const total = collections.reduce((n, c) => n + c.count, 0);
  const all = h(
    "div",
    { class: `item${currentCollection === null ? " selected" : ""}`, onclick: () => void selectCollection(null) },
    h("span", { class: "name" }, "All"),
    h("span", { class: "count" }, String(total)),
  );
  collectionList.replaceChildren(all, ...collections.map(collectionRow));
}

function collectionRow(c: Collection): HTMLElement {
  const row = h(
    "div",
    {
      class: `item${currentCollection === c.id ? " selected" : ""}`,
      draggable: true,
      title: "Double-click to rename. Drag to reorder.",
      onclick: () => void selectCollection(c.id),
      ondblclick: () => renameCollection(c, row),
    },
    h("span", { class: "name" }, c.name),
    h("span", { class: "count" }, String(c.count)),
  );
  row.addEventListener("dragstart", (e) => {
    e.dataTransfer!.setData(DRAG_COLLECTION, String(c.id));
    e.dataTransfer!.effectAllowed = "move";
    row.classList.add("dragging");
  });
  row.addEventListener("dragend", () => row.classList.remove("dragging"));
  row.addEventListener("dragover", (e) => {
    const types = e.dataTransfer?.types ?? [];
    if (types.includes(DRAG_SNIPPET)) {
      e.preventDefault();
      row.classList.add("drop");
    } else if (types.includes(DRAG_COLLECTION)) {
      e.preventDefault();
      row.classList.add("drop-before");
    }
  });
  row.addEventListener("dragleave", () => row.classList.remove("drop", "drop-before"));
  row.addEventListener("drop", (e) => {
    e.preventDefault();
    row.classList.remove("drop", "drop-before");
    const snippetId = e.dataTransfer!.getData(DRAG_SNIPPET);
    const collectionId = e.dataTransfer!.getData(DRAG_COLLECTION);
    if (snippetId) void moveSnippet(Number(snippetId), c.id);
    else if (collectionId) void reorderCollection(Number(collectionId), c.id);
  });
  return row;
}

async function selectCollection(id: number | null) {
  await flush();
  currentCollection = id;
  currentSnippet = null;
  renderCollections();
  await loadSnippets();
}

async function newCollection() {
  let n = 1;
  const names = new Set(collections.map((c) => c.name));
  while (names.has(n === 1 ? "New Collection" : `New Collection ${n}`)) n++;
  try {
    const c = await api.createCollection(n === 1 ? "New Collection" : `New Collection ${n}`);
    await loadCollections();
    await selectCollection(c.id);
    const row = [...collectionList.children].find((el) => el.classList.contains("selected")) as HTMLElement | undefined;
    if (row) renameCollection(c, row);
  } catch (e) {
    await message(errorText(e), { kind: "error" });
  }
}

function renameCollection(c: Collection, row: HTMLElement) {
  const input = h("input", { type: "text", value: c.name });
  let done = false;
  const finish = async (commit: boolean) => {
    if (done) return;
    done = true;
    const name = input.value.trim();
    if (commit && name && name !== c.name) {
      try {
        await api.renameCollection(c.id, name);
      } catch (e) {
        await message(errorText(e), { kind: "error" });
      }
    }
    await loadCollections();
  };
  input.addEventListener("keydown", (e) => {
    e.stopPropagation();
    if (e.key === "Enter") void finish(true);
    if (e.key === "Escape") void finish(false);
  });
  input.addEventListener("blur", () => void finish(true));
  input.addEventListener("click", (e) => e.stopPropagation());
  row.draggable = false;
  row.replaceChildren(input);
  input.focus();
  input.select();
}

async function deleteCurrentCollection() {
  const c = collections.find((x) => x.id === currentCollection);
  if (!c) return;
  const detail = c.count ? ` and its ${c.count} snippet${c.count === 1 ? "" : "s"}` : "";
  if (!(await ask(`Delete the collection “${c.name}”${detail}? This cannot be undone.`, { title: "Delete collection", kind: "warning" }))) return;
  dirty = false;
  await api.deleteCollection(c.id);
  currentCollection = null;
  currentSnippet = null;
  await reload();
}

async function reorderCollection(moved: number, before: number) {
  if (moved === before) return;
  const ids = collections.map((c) => c.id).filter((id) => id !== moved);
  ids.splice(ids.indexOf(before), 0, moved);
  await api.reorderCollections(ids);
  await loadCollections();
}

// ---- snippet list -----------------------------------------------------------

function renderSnippets() {
  if (snippets.length === 0) {
    snippetList.replaceChildren(
      h("div", { class: "placeholder" }, h("span", null, query ? "No snippets match." : "No snippets here yet.")),
    );
    return;
  }
  snippetList.replaceChildren(
    ...snippets.map((s) => {
      const row = h(
        "div",
        {
          class: `item snip${currentSnippet === s.id ? " selected" : ""}`,
          draggable: true,
          dataset: { id: String(s.id) },
          onclick: () => void selectSnippet(s.id),
        },
        h("div", { class: "top" }, h("span", { class: "name" }, s.name), s.keyword && h("span", { class: "kw" }, s.keyword)),
        h("span", { class: "sub" }, s.kind === "image" ? s.body || "Image" : oneLine(s.body, 90) || "Empty"),
      );
      row.addEventListener("dragstart", (e) => {
        e.dataTransfer!.setData(DRAG_SNIPPET, String(s.id));
        e.dataTransfer!.effectAllowed = "move";
      });
      return row;
    }),
  );
}

async function selectSnippet(id: number | null) {
  if (id === currentSnippet) return;
  await flush();
  currentSnippet = id;
  renderSnippets();
  renderEditor();
}

async function moveSnippet(id: number, collectionId: number) {
  await flush();
  await api.updateSnippet(id, { collectionId });
  await reload();
}

async function newSnippet() {
  await flush();
  let collectionId = currentCollection ?? collections[0]?.id;
  if (collectionId === undefined) collectionId = (await api.createCollection("General")).id;
  const s = await api.createSnippet({ collectionId, name: "New Snippet", keyword: null, body: "" });
  query = "";
  filterInput.value = "";
  currentSnippet = s.id;
  await reload();
  const name = editorPane.querySelector<HTMLInputElement>("input[data-field=name]");
  name?.focus();
  name?.select();
}

// ---- editor -----------------------------------------------------------------

function renderEditor() {
  const s = snippets.find((x) => x.id === currentSnippet);
  dirty = false;
  if (!s) {
    editorPane.replaceChildren(
      h(
        "div",
        { class: "placeholder" },
        h("strong", null, snippets.length ? "Select a snippet" : "Your snippet library"),
        h("span", null, "Save any clip with ⌘S in the clipboard viewer, or create a snippet here."),
        h("button", { class: "primary", onclick: () => void newSnippet() }, "New Snippet"),
      ),
    );
    return;
  }

  const name = h("input", { type: "text", value: s.name, dataset: { field: "name" } });
  const keyword = h("input", { type: "text", value: s.keyword ?? "", placeholder: "Reserved for type-to-expand, e.g. ;sig", spellcheck: false });
  const collection = h(
    "select",
    null,
    collections.map((c) => h("option", { value: String(c.id), selected: c.id === s.collectionId }, c.name)),
  );
  const body = h("textarea", { spellcheck: false });
  body.value = s.body;
  const info = h("span", { class: "info" }, usage(s));

  const commit = (): Promise<void> => {
    if (!dirty) return saving;
    dirty = false;
    const patch: SnippetPatch = {
      name: name.value,
      keyword: keyword.value.trim() || null,
      collectionId: Number(collection.value),
    };
    if (s.kind !== "image") patch.body = body.value;
    saving = saving.then(async () => {
      try {
        const updated = await api.updateSnippet(s.id, patch);
        info.className = "info";
        info.textContent = usage(updated);
        const i = snippets.findIndex((x) => x.id === s.id);
        if (i >= 0) snippets[i] = updated;
        renderSnippets();
      } catch (e) {
        info.className = "info error";
        info.textContent = errorText(e);
      }
    });
    return saving;
  };
  flushEditor = commit;

  // Edits autosave on blur; a pause in typing saves too, so nothing is lost.
  const lazy = debounce(() => void commit(), 1200);
  for (const el of [name, keyword, body]) {
    el.addEventListener("input", () => {
      dirty = true;
      lazy();
    });
    el.addEventListener("blur", () => void commit());
  }
  collection.addEventListener("change", () => {
    dirty = true;
    void commit().then(reload);
  });

  const insert = (token: string) => {
    body.focus();
    body.setRangeText(token, body.selectionStart, body.selectionEnd, "end");
    dirty = true;
    lazy();
  };

  const bodyArea =
    s.kind === "image"
      ? h("div", { class: "bodywrap" }, h("span", null, "Image"), h("div", { class: "imagebox" }, h("img", { src: s.imageUrl, alt: s.name })))
      : h(
          "div",
          { class: "bodywrap" },
          h("span", null, "Body"),
          body,
          h("div", { class: "chips" }, "Insert:", PLACEHOLDERS.map((p) => h("button", { title: PLACEHOLDER_HELP[p], onclick: () => insert(p) }, p))),
        );

  editorPane.replaceChildren(
    h(
      "div",
      { class: "editor" },
      h("label", { class: "field" }, h("span", null, "Name"), name),
      h("label", { class: "field" }, h("span", null, "Keyword"), keyword),
      h("label", { class: "field" }, h("span", null, "Collection"), collection),
      bodyArea,
      h(
        "div",
        { class: "bar" },
        info,
        h("button", { onclick: () => void copySnippet(s.id, info) }, "Copy to Clipboard"),
        h("button", { onclick: () => void duplicateSnippet(s.id) }, "Duplicate"),
        h("button", { class: "danger", onclick: () => void deleteSnippet(s) }, "Delete"),
      ),
    ),
  );
}

const PLACEHOLDER_HELP: Record<string, string> = {
  "{date}": "Today's date. Custom format: {date:%d %B %Y}",
  "{time}": "Current time. Custom format: {time:%H:%M:%S}",
  "{clipboard}": "Current clipboard text. {clipboard:1} is the previous clip",
  "{cursor}": "Where the caret ends up after pasting",
};

let flushEditor: () => Promise<void> = () => Promise.resolve();

/** Save pending edits before the editor is replaced. */
async function flush() {
  await flushEditor();
  await saving;
}

function usage(s: Snippet): string {
  return [`Used ${s.useCount}×`, s.originApp && `from ${s.originApp}`].filter(Boolean).join(" · ");
}

async function copySnippet(id: number, info: HTMLElement) {
  await flush();
  try {
    await api.copy({ snippet: id });
    info.className = "info";
    info.textContent = "Copied to clipboard";
  } catch (e) {
    info.className = "info error";
    info.textContent = errorText(e);
  }
}

async function duplicateSnippet(id: number) {
  await flush();
  const copy = await api.duplicateSnippet(id);
  currentSnippet = copy.id;
  await reload();
}

async function deleteSnippet(s: Snippet) {
  if (!(await ask(`Delete the snippet “${s.name}”?`, { title: "Delete snippet", kind: "warning" }))) return;
  dirty = false;
  await api.deleteSnippet(s.id);
  currentSnippet = null;
  await reload();
}

// ---- import / export (design section 8.4) -----------------------------------

async function exportCollection() {
  await flush();
  const c = collections.find((x) => x.id === currentCollection);
  if (!c) {
    await message("Select a collection to export. Each collection is exported as its own JSON file.", { title: "Export" });
    return;
  }
  const path = await save({ defaultPath: `${c.name}.json`, filters: [{ name: "JSON", extensions: ["json"] }] });
  if (!path) return;
  try {
    await api.exportCollection(c.id, path);
  } catch (e) {
    await message(errorText(e), { kind: "error" });
  }
}

async function importCollection() {
  await flush();
  const picked = await open({ multiple: true, filters: [{ name: "JSON", extensions: ["json"] }] });
  if (!picked) return;
  for (const path of Array.isArray(picked) ? picked : [picked]) {
    try {
      let report = await api.importCollection(path, "skip");
      if (report.conflicts.length) {
        const names = report.conflicts.slice(0, 8).join(", ") + (report.conflicts.length > 8 ? ", …" : "");
        const n = report.conflicts.length;
        const overwrite = await ask(
          `${n} snippet${n === 1 ? "" : "s"} in “${report.collection}” already exist${n === 1 ? "s" : ""}: ${names}.\n\nOverwrite ${n === 1 ? "it" : "them"} with the imported version?`,
          { title: "Import", kind: "warning", okLabel: "Overwrite", cancelLabel: "Keep Existing" },
        );
        if (overwrite) report = await api.importCollection(path, "overwrite");
      }
      currentCollection = report.collectionId;
    } catch (e) {
      await message(errorText(e), { title: "Import failed", kind: "error" });
    }
  }
  currentSnippet = null;
  await reload();
}

// ---- data -------------------------------------------------------------------

async function loadCollections() {
  collections = await api.listCollections();
  if (currentCollection !== null && !collections.some((c) => c.id === currentCollection)) currentCollection = null;
  renderCollections();
}

async function loadSnippets() {
  snippets = await api.listSnippets(currentCollection, query);
  if (currentSnippet !== null && !snippets.some((s) => s.id === currentSnippet)) currentSnippet = null;
  renderSnippets();
  renderEditor();
}

async function reload() {
  await loadCollections();
  await loadSnippets();
}

/** Refresh after a change made elsewhere, without disturbing an edit in progress. */
async function externalChange() {
  if (dirty || editorPane.contains(document.activeElement)) {
    collections = await api.listCollections();
    renderCollections();
    return;
  }
  await reload();
}

filterInput.addEventListener(
  "input",
  debounce(() => {
    query = filterInput.value;
    void flush().then(loadSnippets);
  }, 80),
);

window.addEventListener("keydown", (e) => {
  if (!e.metaKey) return;
  if (e.code === "KeyN") {
    e.preventDefault();
    void (e.shiftKey ? newCollection() : newSnippet());
  } else if (e.code === "KeyF") {
    e.preventDefault();
    filterInput.focus();
  } else if (e.key === "Backspace" && !editorPane.contains(document.activeElement) && document.activeElement !== filterInput) {
    e.preventDefault();
    const s = snippets.find((x) => x.id === currentSnippet);
    if (s) void deleteSnippet(s);
    else void deleteCurrentCollection();
  }
});
window.addEventListener("blur", () => void flush());

void on("snippets-changed", () => void externalChange());
void reload();
