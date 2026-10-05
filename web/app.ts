// The notes page: search, tag filters, the timeline list, and the bridge to
// the WebGPU scene (src/lib.rs). The list works on its own; the scene is added
// when the browser supports WebGPU.

export {};

interface TagInfo {
  name: string;
  color: string;
  count: number;
}

interface Manifest {
  build: string;
  /** How many data/slugs files there are. */
  slugBuckets: number;
  count: number;
  chunk: number;
  defaultColor: string;
  tags: TagInfo[];
}

/**
 * One row of data/notes/<chunk>.json: slug, title, date, tags, summary, and
 * the language when it is not English.
 */
type NoteRow = [string, string, string, number[], string, string?];

interface Note {
  id: number;
  slug: string;
  title: string;
  date: string;
  tags: number[];
  summary: string;
  lang: string;
}

interface SceneBindings {
  default(init: { module_or_path: string }): Promise<unknown>;
  load_scene(bytes: Uint8Array): void;
  set_matches(ids: Uint32Array): void;
  clear_matches(): void;
  select(id: number): void;
  set_reduced_motion(reduced: boolean): void;
  set_paused(paused: boolean): void;
  set_insets(top: number, right: number, bottom: number, left: number): void;
  labels(): Float32Array;
}

interface SearchResult {
  id: string;
  data(): Promise<{ excerpt: string }>;
}

interface Pagefind {
  init(): Promise<void>;
  search(term: string): Promise<{ results: SearchResult[] }>;
}

type View = "list" | "3d";

const MONTHS = ["January", "February", "March", "April", "May", "June", "July", "August", "September",
  "October", "November", "December"];
const LIST_BATCH = 20;
const SEARCH_DELAY_MS = 180;
const SCENE_TIMEOUT_MS = 15000;
/** Set on <html> by the script in the page's head; see build.py. */
const WAITING_FOR_SCENE = "waiting-for-scene";
const LABEL_STRIDE = 5;
const NO_TAG = 0xffff;
const TAGS_PER_NOTE = 5;
const INDEX_HEADER_BYTES = 12;
const INDEX_RECORD_BYTES = 4 + TAGS_PER_NOTE * 2;

const root = document.documentElement;
const base = root.dataset.base ?? "/";
const build = root.dataset.build ?? "dev";

function element<T extends HTMLElement>(id: string): T {
  const found = document.getElementById(id);
  if (!found) {
    throw new Error(`The page has no #${id}`);
  }
  return found as T;
}

const searchForm = element<HTMLFormElement>("search");
const searchInput = element<HTMLInputElement>("search-input");
const searchStatus = element<HTMLElement>("search-status");
const viewSwitch = element<HTMLButtonElement>("view-switch");
const tagBar = element<HTMLElement>("tag-bar");
const sceneMount = element<HTMLElement>("scene");
const labelLayer = element<HTMLElement>("scene-labels");
const preview = element<HTMLElement>("preview");
const listSummary = element<HTMLElement>("list-summary");
const listItems = element<HTMLElement>("list-items");
const listMore = element<HTMLElement>("list-more");
const toolbar = document.querySelector<HTMLElement>(".toolbar")!;

const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
const narrowScreen = window.matchMedia("(max-width: 620px)");

let manifest: Manifest;
let view: View = "list";
let scene: SceneBindings | null = null;
let query = "";
const activeTags = new Set<number>();
/** Matching notes, newest first, or null when nothing is filtered. */
let matches: Uint32Array | null = null;
let excerpts = new Map<number, SearchResult>();
let selected = -1;

// ------------------------------------------------------------------- data

function asset(path: string): string {
  return `${base}${path}?v=${build}`;
}

async function fetchJson<T>(path: string): Promise<T> {
  const response = await fetch(asset(path));
  if (!response.ok) {
    throw new Error(`${path}: ${response.status}`);
  }
  return (await response.json()) as T;
}

async function fetchBytes(path: string): Promise<ArrayBuffer> {
  const response = await fetch(asset(path));
  if (!response.ok) {
    throw new Error(`${path}: ${response.status}`);
  }
  return response.arrayBuffer();
}

const chunks = new Map<number, Promise<Note[]>>();

function loadChunk(chunk: number): Promise<Note[]> {
  let pending = chunks.get(chunk);
  if (!pending) {
    pending = fetchJson<NoteRow[]>(`data/notes/${chunk}.json`).then((rows) =>
      rows.map(([slug, title, date, tags, summary, lang], offset) => ({
        id: chunk * manifest.chunk + offset,
        slug,
        title,
        date,
        tags,
        summary,
        lang: lang ?? "en",
      })),
    );
    pending.catch(() => chunks.delete(chunk));
    chunks.set(chunk, pending);
  }
  return pending;
}

const loadedNotes = new Map<number, Note>();

async function loadNote(id: number): Promise<Note | undefined> {
  const known = loadedNotes.get(id);
  if (known) {
    return known;
  }
  const notes = await loadChunk(Math.floor(id / manifest.chunk));
  for (const note of notes) {
    loadedNotes.set(note.id, note);
  }
  return loadedNotes.get(id);
}

/** Which data/slugs file holds a slug. `slug_bucket` in build.py matches it. */
function slugBucket(slug: string, buckets: number): number {
  let value = 0x811c9dc5; // 32-bit FNV-1a
  for (const byte of new TextEncoder().encode(slug)) {
    value = Math.imul(value ^ byte, 0x01000193) >>> 0;
  }
  return value % buckets;
}

/**
 * The number of the note a "?note=" link names, or -1. The link carries the
 * note's address, which never changes. Its number does: it moves whenever a
 * note with an earlier date is added.
 */
async function findNote(wanted: string): Promise<number> {
  if (!wanted) {
    return -1;
  }
  try {
    const bucket = slugBucket(wanted, Math.max(1, manifest.slugBuckets));
    const slugs = await fetchJson<Record<string, number>>(`data/slugs/${bucket}.json`);
    return slugs[wanted] ?? -1;
  } catch (error) {
    console.error(error);
    return -1;
  }
}

let tagIndex: Promise<Uint16Array> | null = null;

/** Up to five tag numbers for every note, from data/index.bin. */
function loadTagIndex(): Promise<Uint16Array> {
  tagIndex ??= fetchBytes("data/index.bin").then((buffer) => {
    const data = new DataView(buffer);
    const count = data.getUint32(8, true);
    const tags = new Uint16Array(count * TAGS_PER_NOTE);
    for (let note = 0; note < count; note++) {
      const record = INDEX_HEADER_BYTES + note * INDEX_RECORD_BYTES;
      for (let slot = 0; slot < TAGS_PER_NOTE; slot++) {
        tags[note * TAGS_PER_NOTE + slot] = data.getUint16(record + 4 + slot * 2, true);
      }
    }
    return tags;
  });
  return tagIndex;
}

function hasTags(tags: Uint16Array, note: number, wanted: number[]): boolean {
  return wanted.every((tag) => {
    for (let slot = 0; slot < TAGS_PER_NOTE; slot++) {
      if (tags[note * TAGS_PER_NOTE + slot] === tag) {
        return true;
      }
    }
    return false;
  });
}

let searchEngine: Promise<{ pagefind: Pagefind; ids: Record<string, number> }> | null = null;

function loadSearch(): Promise<{ pagefind: Pagefind; ids: Record<string, number> }> {
  searchEngine ??= (async () => {
    const [pagefind, ids] = await Promise.all([
      import(`${base}pagefind/pagefind.js`) as Promise<Pagefind>,
      fetchJson<Record<string, number>>("data/search-map.json"),
    ]);
    await pagefind.init();
    return { pagefind, ids };
  })();
  searchEngine.catch(() => {
    searchEngine = null;
  });
  return searchEngine;
}

// ---------------------------------------------------------------- filters

let filterRun = 0;

async function applyFilters(): Promise<void> {
  const run = ++filterRun;
  const wanted = [...activeTags];
  let found: number[] | null = null;
  const results = new Map<number, SearchResult>();

  try {
    if (query) {
      const { pagefind, ids } = await loadSearch();
      const search = await pagefind.search(query);
      found = [];
      for (const result of search.results) {
        const id = ids[result.id];
        if (id !== undefined) {
          found.push(id);
          results.set(id, result);
        }
      }
    }
    if (wanted.length > 0) {
      const tags = await loadTagIndex();
      if (found) {
        found = found.filter((id) => hasTags(tags, id, wanted));
      } else {
        found = [];
        for (let id = 0; id < manifest.count; id++) {
          if (hasTags(tags, id, wanted)) {
            found.push(id);
          }
        }
      }
    }
  } catch (error) {
    console.error(error);
    if (run === filterRun) {
      searchStatus.textContent = "Search is unavailable";
    }
    return;
  }
  if (run !== filterRun) {
    return;
  }

  matches = found ? Uint32Array.from(found).sort().reverse() : null;
  excerpts = results;
  if (scene) {
    if (matches) {
      scene.set_matches(matches);
    } else {
      scene.clear_matches();
    }
  }
  if (selected >= 0 && matches && !matches.includes(selected)) {
    closePreview();
  }
  showStatus();
  renderList();
  rememberInAddress();
}

function showStatus(): void {
  const total = manifest.count;
  const plural = total === 1 ? "" : "s";
  if (!matches) {
    searchStatus.textContent = "";
    listSummary.textContent = `${total} note${plural}, newest first`;
  } else if (matches.length === 0) {
    searchStatus.textContent = `0 / ${total}`;
    listSummary.textContent = "No notes match. Try a different word or clear a tag.";
  } else {
    searchStatus.textContent = `${matches.length} / ${total}`;
    listSummary.textContent = `${matches.length} of ${total} note${plural} match`;
  }
}

function rememberInAddress(): void {
  const parameters = new URLSearchParams();
  if (query) {
    parameters.set("q", query);
  }
  for (const tag of activeTags) {
    const info = manifest.tags[tag];
    if (info) {
      parameters.append("tag", info.name);
    }
  }
  if (view === "list" && scene) {
    parameters.set("view", "list");
  }
  const text = parameters.toString();
  history.replaceState(null, "", text ? `${base}?${text}` : base);
}

// ------------------------------------------------------------------- list

let listRun = 0;
let rendered = 0;
let lastYear = "";
let listBusy = false;

function listLength(): number {
  return matches ? matches.length : manifest.count;
}

function listNote(position: number): number {
  return matches ? matches[position]! : manifest.count - 1 - position;
}

function tagLink(tag: number): HTMLAnchorElement | null {
  const info = manifest.tags[tag];
  if (!info) {
    return null;
  }
  const link = document.createElement("a");
  link.className = "tag";
  link.dir = "auto";
  link.href = `${base}tags/${encodeURIComponent(info.name)}/`;
  link.style.setProperty("--tag", info.color);
  link.textContent = `#${info.name}`;
  return link;
}

function tagList(note: Note): HTMLUListElement | null {
  if (note.tags.length === 0) {
    return null;
  }
  const list = document.createElement("ul");
  list.className = "note-tags";
  list.setAttribute("aria-label", "Tags");
  for (const tag of note.tags) {
    const link = tagLink(tag);
    if (link) {
      const item = document.createElement("li");
      item.append(link);
      list.append(item);
    }
  }
  return list;
}

/**
 * Lets the browser lay a title or summary out in its own direction, so a
 * Persian note reads right to left among English ones.
 */
function inItsLanguage<T extends HTMLElement>(target: T, note: Note): T {
  target.dir = "auto";
  if (note.lang !== "en") {
    target.lang = note.lang;
  }
  return target;
}

function shortDate(date: string): string {
  const [, month, day] = date.split("-").map(Number);
  return `${MONTHS[(month ?? 1) - 1]!.slice(0, 3)} ${day}`;
}

function longDate(date: string): string {
  const [year, month, day] = date.split("-").map(Number);
  return `${MONTHS[(month ?? 1) - 1]} ${day}, ${year}`;
}

function noteItem(note: Note): HTMLElement {
  const address = `${base}${note.slug}/`;
  const item = document.createElement("article");
  item.className = "note-item";
  item.dataset.id = String(note.id);

  const time = document.createElement("time");
  time.dateTime = note.date;
  time.textContent = shortDate(note.date);

  const heading = inItsLanguage(document.createElement("h3"), note);
  const title = document.createElement("a");
  title.href = address;
  title.textContent = note.title;
  heading.append(title);

  const summary = inItsLanguage(document.createElement("p"), note);
  summary.textContent = note.summary;
  const result = excerpts.get(note.id);
  if (result) {
    // Pagefind's excerpt is escaped text with <mark> around the matched words.
    void result.data().then((data) => {
      if (summary.isConnected && data.excerpt) {
        summary.innerHTML = data.excerpt;
      }
    });
  }

  const actions = document.createElement("div");
  actions.className = "note-actions";
  const read = document.createElement("a");
  read.className = "btn";
  read.href = address;
  read.textContent = "Read more";
  read.setAttribute("aria-label", `Read more: ${note.title}`);
  const locate = document.createElement("button");
  locate.className = "btn";
  locate.type = "button";
  locate.dataset.locate = String(note.id);
  locate.textContent = "Locate in 3D";
  locate.hidden = !scene;
  actions.append(read, locate);

  const body = document.createElement("div");
  body.append(heading, summary);
  const tags = tagList(note);
  if (tags) {
    body.append(tags);
  }
  body.append(actions);
  item.append(time, body);
  return item;
}

async function renderMore(): Promise<void> {
  if (listBusy || rendered >= listLength()) {
    return;
  }
  listBusy = true;
  const run = listRun;
  let failed = false;
  listMore.textContent = "Loading more notes…";
  try {
    const end = Math.min(rendered + LIST_BATCH, listLength());
    const ids: number[] = [];
    for (let position = rendered; position < end; position++) {
      ids.push(listNote(position));
    }
    const notes = await Promise.all(ids.map(loadNote));
    if (run !== listRun) {
      return;
    }
    const fragment = document.createDocumentFragment();
    for (const note of notes) {
      if (!note) {
        continue;
      }
      const year = note.date.slice(0, 4);
      if (year !== lastYear) {
        lastYear = year;
        const divider = document.createElement("div");
        divider.className = "year-divider";
        divider.setAttribute("role", "heading");
        divider.setAttribute("aria-level", "2");
        divider.textContent = year;
        fragment.append(divider);
      }
      fragment.append(noteItem(note));
    }
    listItems.append(fragment);
    rendered = end;
  } catch (error) {
    console.error(error);
    failed = true;
  } finally {
    listBusy = false;
    if (run !== listRun) {
      void renderMore();
    } else if (failed) {
      // Do not try again straight away: with the network down that would
      // repeat without pause. The next scroll tries again.
      listMore.textContent = "Could not load more notes. Scroll to try again.";
    } else {
      listMore.textContent = rendered >= listLength() && rendered > LIST_BATCH ? "End of timeline" : "";
      // Keep going while the end of the list is still on screen.
      if (view === "list" && rendered < listLength() && nearEndOfList()) {
        void renderMore();
      }
    }
  }
}

function nearEndOfList(): boolean {
  return listMore.getBoundingClientRect().top < window.innerHeight + 600;
}

function renderList(): void {
  listRun++;
  rendered = 0;
  lastYear = "";
  listItems.textContent = "";
  listMore.textContent = "";
  void renderMore();
}

/** The newest notes are already in the page; carry on from the last of them. */
function adoptPrerenderedList(): void {
  const items = listItems.querySelectorAll<HTMLElement>(".note-item");
  rendered = items.length;
  const last = items[items.length - 1]?.querySelector("time");
  lastYear = last?.dateTime.slice(0, 4) ?? "";
}

// ------------------------------------------------------------------- tags

function setTag(tag: number, active: boolean): void {
  if (active) {
    activeTags.add(tag);
  } else {
    activeTags.delete(tag);
  }
  let chip = tagBar.querySelector<HTMLElement>(`[data-tag="${tag}"]`);
  if (!chip && active) {
    // A tag from the address that is too rare to be in the bar.
    const link = tagLink(tag);
    if (link) {
      link.dataset.tag = String(tag);
      link.setAttribute("role", "button");
      tagBar.prepend(link);
      chip = link;
    }
  }
  chip?.setAttribute("aria-pressed", String(active));
  tagBar.classList.toggle("is-filtering", activeTags.size > 0);
}

// ---------------------------------------------------------------- preview

function closePreview(): void {
  if (selected < 0 && preview.hidden) {
    return;
  }
  selected = -1;
  preview.hidden = true;
  preview.textContent = "";
  scene?.select(-1);
  updateInsets();
}

async function openPreview(id: number): Promise<void> {
  selected = id;
  const note = await loadNote(id).catch(() => undefined);
  if (!note || selected !== id) {
    return;
  }
  const eyebrow = document.createElement("p");
  eyebrow.className = "eyebrow";
  eyebrow.textContent = longDate(note.date);

  const heading = inItsLanguage(document.createElement("h2"), note);
  heading.textContent = note.title;

  const summary = inItsLanguage(document.createElement("p"), note);
  summary.textContent = note.summary;

  const actions = document.createElement("div");
  actions.className = "note-actions";
  const read = document.createElement("a");
  read.className = "btn";
  read.href = `${base}${note.slug}/`;
  read.textContent = "Read more";
  const close = document.createElement("button");
  close.className = "btn";
  close.type = "button";
  close.textContent = "Close";
  close.addEventListener("click", closePreview);
  actions.append(read, close);

  preview.textContent = "";
  preview.append(eyebrow, heading, summary);
  const tags = tagList(note);
  if (tags) {
    preview.append(tags);
  }
  preview.append(actions);
  preview.hidden = false;
  updateInsets();
}

// ------------------------------------------------------------------ scene

/** Tells the scene which part of the canvas is free of interface. */
function updateInsets(): void {
  if (!scene) {
    return;
  }
  const top = toolbar.getBoundingClientRect().bottom;
  let right = 0;
  let bottom = 0;
  if (!preview.hidden && view === "3d") {
    const panel = preview.getBoundingClientRect();
    if (narrowScreen.matches) {
      bottom = window.innerHeight - panel.top;
    } else {
      right = window.innerWidth - panel.left;
    }
  }
  scene.set_insets(top, right, bottom, 0);
}

const labels = new Map<number, HTMLElement>();
const labelWidths = new Map<number, number>();
const reticles: HTMLElement[] = [];
const LABEL_HEIGHT = 22;
/** Hidden labels kept ready for reuse. Beyond this many they are removed. */
const LABEL_POOL = 96;

function reticle(index: number): HTMLElement {
  let found = reticles[index];
  if (!found) {
    found = document.createElement("div");
    found.className = "scene-reticle";
    found.append(...[0, 1, 2, 3].map(() => document.createElement("i")));
    labelLayer.append(found);
    reticles[index] = found;
  }
  return found;
}

function labelFor(id: number): HTMLElement | null {
  const known = labels.get(id);
  if (known) {
    return known;
  }
  const note = loadedNotes.get(id);
  if (!note) {
    void loadNote(id).catch(() => undefined);
    return null;
  }
  // A button, so the label can be clicked like the orb it names.
  const label = document.createElement("button");
  label.type = "button";
  label.className = "scene-label";
  label.dataset.note = String(id);
  label.tabIndex = -1;
  if (note.lang !== "en") {
    label.lang = note.lang;
  }
  label.textContent = note.title;
  const first = note.tags[0];
  label.style.setProperty("--tag", (first !== undefined && first !== NO_TAG ? manifest.tags[first]?.color : undefined)
    ?? manifest.defaultColor);
  labelLayer.append(label);
  labels.set(id, label);
  labelWidths.set(id, label.offsetWidth);
  return label;
}

function placeLabels(): void {
  if (view === "3d" && scene) {
    const data = scene.labels();
    const width = window.innerWidth;
    const top = toolbar.getBoundingClientRect().bottom;
    const panel = preview.hidden ? null : preview.getBoundingClientRect();
    const limit = narrowScreen.matches ? 6 : 32;
    const placed: DOMRect[] = [];
    const shown = new Set<number>();
    let reticleCount = 0;

    for (let at = 0; at + LABEL_STRIDE <= data.length; at += LABEL_STRIDE) {
      const id = data[at]!;
      const x = data[at + 1]!;
      const y = data[at + 2]!;
      const radius = data[at + 3]!;
      const pinned = data[at + 4]! > 0;

      if (pinned) {
        const mark = reticle(reticleCount++);
        mark.style.display = "";
        mark.style.setProperty("--r", `${radius + 9}px`);
        mark.style.transform = `translate(${x}px, ${y}px)`;
      }
      if (!pinned && shown.size >= limit) {
        continue;
      }
      const label = labelFor(id);
      if (!label) {
        continue;
      }
      const labelWidth = labelWidths.get(id) ?? 0;
      let left = x + radius + 16;
      if (left + labelWidth > width - 8) {
        left = x - radius - 16 - labelWidth;
      }
      let box = new DOMRect(left, y - 12 - LABEL_HEIGHT / 2, labelWidth, LABEL_HEIGHT);
      if (panel !== null && overlaps(box, panel)) {
        // The preview is in the way; try the other side of the orb.
        box = new DOMRect(x - radius - 16 - labelWidth, box.top, labelWidth, LABEL_HEIGHT);
      }
      if (box.top < top + 4 || box.left < 4) {
        continue;
      }
      const covered = panel !== null && overlaps(box, panel);
      const crowded = placed.some((other) => overlaps(box, other));
      if (covered || (crowded && !pinned)) {
        continue;
      }
      placed.push(box);
      shown.add(id);
      label.classList.toggle("is-active", pinned);
      label.style.display = "";
      label.style.transform = `translate(${Math.round(box.left)}px, ${Math.round(box.top)}px)`;
    }

    for (const [id, label] of labels) {
      if (shown.has(id)) {
        continue;
      }
      if (labels.size > LABEL_POOL) {
        // Moving through thousands of notes would otherwise leave a hidden
        // label behind for every one of them.
        label.remove();
        labels.delete(id);
        labelWidths.delete(id);
      } else {
        label.style.display = "none";
      }
    }
    for (let index = reticleCount; index < reticles.length; index++) {
      reticles[index]!.style.display = "none";
    }
  }
  requestAnimationFrame(placeLabels);
}

function overlaps(a: DOMRect, b: DOMRect): boolean {
  return a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top;
}

function waitForEvent(name: string, milliseconds: number): Promise<void> {
  return new Promise((resolve, reject) => {
    const timer = window.setTimeout(() => reject(new Error(`${name} did not arrive`)), milliseconds);
    window.addEventListener(name, () => {
      window.clearTimeout(timer);
      resolve();
    }, { once: true });
  });
}

async function hasWebGpu(): Promise<boolean> {
  const gpu = (navigator as Navigator & { gpu?: { requestAdapter(): Promise<unknown> } }).gpu;
  if (!gpu) {
    return false;
  }
  try {
    return (await gpu.requestAdapter()) !== null;
  } catch {
    return false;
  }
}

async function startScene(): Promise<SceneBindings> {
  const ready = waitForEvent("notes:ready", SCENE_TIMEOUT_MS);
  // If loading fails first, nobody waits for this promise any more.
  ready.catch(() => undefined);
  const [bindings, bytes] = await Promise.all([
    import(`${base}pkg/pooya_notes.js?v=${build}`) as Promise<SceneBindings>,
    fetchBytes("data/scene.bin"),
  ]);
  await bindings.default({ module_or_path: asset("pkg/pooya_notes_bg.wasm") });
  await ready;
  bindings.set_reduced_motion(reducedMotion.matches);
  bindings.load_scene(new Uint8Array(bytes));
  return bindings;
}

function setView(next: View): void {
  if (next === "3d" && !scene) {
    next = "list";
  }
  // The page no longer waits for the scene.
  root.classList.remove(WAITING_FOR_SCENE);
  view = next;
  document.body.classList.toggle("view-3d", view === "3d");
  document.body.classList.toggle("view-list", view === "list");
  viewSwitch.textContent = view === "3d" ? "Show list" : "Show 3D";
  scene?.set_paused(view !== "3d");
  if (view === "3d") {
    window.scrollTo(0, 0);
    updateInsets();
  } else if (rendered < listLength() && nearEndOfList()) {
    void renderMore();
  }
  rememberInAddress();
}

// ------------------------------------------------------------------ start

function listen(): void {
  let timer = 0;
  searchInput.addEventListener("input", () => {
    window.clearTimeout(timer);
    timer = window.setTimeout(() => {
      query = searchInput.value.trim();
      void applyFilters();
    }, SEARCH_DELAY_MS);
  });
  searchInput.addEventListener("focus", () => void loadSearch().catch(() => undefined), { once: true });
  searchForm.addEventListener("submit", (event) => {
    event.preventDefault();
    window.clearTimeout(timer);
    query = searchInput.value.trim();
    void applyFilters();
  });

  tagBar.addEventListener("click", (event) => {
    const chip = (event.target as HTMLElement).closest<HTMLElement>("[data-tag]");
    if (!chip) {
      return;
    }
    event.preventDefault();
    const tag = Number(chip.dataset.tag);
    setTag(tag, !activeTags.has(tag));
    void applyFilters();
  });
  tagBar.addEventListener("keydown", (event) => {
    if (event.key === " " && (event.target as HTMLElement).matches("[data-tag]")) {
      event.preventDefault();
      (event.target as HTMLElement).click();
    }
  });

  labelLayer.addEventListener("click", (event) => {
    const label = (event.target as HTMLElement).closest<HTMLElement>("[data-note]");
    if (label && scene) {
      scene.select(Number(label.dataset.note));
    }
  });

  viewSwitch.addEventListener("click", () => {
    setView(view === "3d" ? "list" : "3d");
  });

  listItems.addEventListener("click", (event) => {
    const button = (event.target as HTMLElement).closest<HTMLElement>("[data-locate]");
    if (button && scene) {
      setView("3d");
      scene.select(Number(button.dataset.locate));
    }
  });

  window.addEventListener("scroll", () => {
    if (view === "list" && nearEndOfList()) {
      void renderMore();
    }
  }, { passive: true });
  window.addEventListener("resize", updateInsets);

  window.addEventListener("keydown", (event) => {
    const typing = event.target instanceof HTMLInputElement;
    if (event.key === "/" && !typing) {
      event.preventDefault();
      searchInput.focus();
    } else if (event.key === "Escape") {
      if (selected >= 0) {
        closePreview();
      } else if (typing) {
        searchInput.blur();
      }
    }
  });

  window.addEventListener("notes:select", (event) => {
    const id = (event as CustomEvent<number>).detail;
    if (id >= 0) {
      void openPreview(id);
    } else if (selected >= 0) {
      selected = -1;
      preview.hidden = true;
      preview.textContent = "";
      updateInsets();
    }
  });

  reducedMotion.addEventListener("change", () => scene?.set_reduced_motion(reducedMotion.matches));
}

async function start(): Promise<void> {
  // The page opens on the scene. A script in the head has already hidden the
  // list if this browser has WebGPU, so the list never flashes by first. The
  // scene starts loading at once, alongside everything else.
  const waiting = root.classList.contains(WAITING_FOR_SCENE);
  const starting: Promise<SceneBindings | null> = hasWebGpu()
    .then((supported) => (supported ? startScene() : null))
    .catch((error) => {
      console.error(error);
      sceneMount.querySelector("canvas")?.remove();
      return null;
    });

  adoptPrerenderedList();
  listen();
  manifest = await fetchJson<Manifest>("data/manifest.json");

  const parameters = new URLSearchParams(location.search);
  query = (parameters.get("q") ?? "").trim();
  searchInput.value = query;
  for (const name of parameters.getAll("tag")) {
    const tag = manifest.tags.findIndex((info) => info.name === name);
    if (tag >= 0) {
      setTag(tag, true);
    }
  }
  const wantsList = parameters.get("view") === "list";
  const wantedNote = findNote(parameters.get("note") ?? "");
  if (query || activeTags.size > 0) {
    await applyFilters();
  }

  scene = await starting;
  if (!scene) {
    // No WebGPU, or the scene failed to start: the list is the site.
    setView("list");
    return;
  }
  if (matches) {
    scene.set_matches(matches);
  }
  showStatus();
  viewSwitch.hidden = false;
  for (const button of listItems.querySelectorAll<HTMLElement>("[data-locate]")) {
    button.hidden = false;
  }
  requestAnimationFrame(placeLabels);

  // The scene is the default view, unless the address asks for the list.
  const located = await wantedNote;
  if (located >= 0 || (waiting && !wantsList)) {
    setView("3d");
    if (located >= 0) {
      scene.select(located);
    }
  } else {
    setView("list");
  }
}

start().catch((error) => {
  console.error(error);
  root.classList.remove(WAITING_FOR_SCENE);
});
