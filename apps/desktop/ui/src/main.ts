import "./styles.css";

import { loadDocument, loadLibrary, saveDocument } from "./api";
import { NoteViewer, type ViewerMode } from "./editor";
import { renderFallback } from "./fallback";
import { allBooks } from "./projection";
import { mountLibraryScene, type SceneHandle } from "./scene";
import type { CommandError, DesktopLibrary, LibraryBook } from "./types";

const form = required<HTMLFormElement>("library-form");
const rootInput = required<HTMLInputElement>("root-input");
const projectInput = required<HTMLInputElement>("project-input");
const status = required<HTMLElement>("status");
const sceneHost = required<HTMLElement>("scene");
const fallbackHost = required<HTMLElement>("fallback");
const metaHost = required<HTMLElement>("book-meta");
const editActions = required<HTMLElement>("edit-actions");
const editState = required<HTMLElement>("edit-state");
const saveButton = required<HTMLButtonElement>("save-note");
const discardButton = required<HTMLButtonElement>("discard-note");
let viewer: NoteViewer;
viewer = new NoteViewer(required<HTMLElement>("note-viewer"), updateEditActions);
const reducedMotion = required<HTMLInputElement>("reduced-motion");
const modeButtons = [...document.querySelectorAll<HTMLButtonElement>("[data-mode]")];

let library: DesktopLibrary | null = null;
let scene: SceneHandle | null = null;
let selectedId: string | null = null;
let activeResolution: { root: string; project: string } | null = null;

reducedMotion.checked = window.matchMedia("(prefers-reduced-motion: reduce)").matches;

form.addEventListener("submit", (event) => {
  event.preventDefault();
  if (viewer.dirty) {
    requireDirtyDecision("opening another library");
    return;
  }
  void openLibrary();
});

saveButton.addEventListener("click", () => void saveSelectedNote());
discardButton.addEventListener("click", () => {
  viewer.discard();
  setStatus("Changes discarded. You can navigate away.", "success");
});

reducedMotion.addEventListener("change", () => {
  if (library) {
    void renderScene(library);
  }
});

for (const button of modeButtons) {
  button.addEventListener("click", () => {
    const mode = button.dataset.mode as ViewerMode;
    viewer.setMode(mode);
    for (const candidate of modeButtons) {
      candidate.setAttribute("aria-selected", String(candidate === button));
    }
  });
}

void openLibrary();

async function openLibrary(
  preferredId?: string,
  requestedResolution = { root: rootInput.value, project: projectInput.value },
): Promise<void> {
  setStatus("Validating the synthetic library through Akasha Core…");
  form.classList.add("is-loading");
  try {
    library = await loadLibrary(requestedResolution.root, requestedResolution.project);
    activeResolution = requestedResolution;
    rootInput.value = requestedResolution.root;
    projectInput.value = requestedResolution.project;
    selectedId = null;
    renderFallback(fallbackHost, library.projection, (book) => void selectBook(book));
    await renderScene(library);
    const books = allBooks(library.projection);
    const preferred = books.find((book) => book.id === preferredId);
    const initial = preferred ?? books[0];
    if (initial) {
      await selectBook(initial);
    }
    const recovery = library.recovery === "none" ? "" : ` / recovery ${library.recovery}`;
    setStatus(
      `${library.projection.total_books} canonical books / ${library.projection.projects.length} project shelves / validation passed${recovery}`,
      "success",
    );
  } catch (error) {
    library = null;
    activeResolution = null;
    scene?.destroy();
    scene = null;
    sceneHost.replaceChildren();
    fallbackHost.replaceChildren();
    metaHost.textContent = errorMessage(error);
    viewer.setDocument("", false);
    setStatus(`Library unavailable: ${errorMessage(error)}`, "error");
  } finally {
    form.classList.remove("is-loading");
  }
}

async function renderScene(current: DesktopLibrary): Promise<void> {
  scene?.destroy();
  scene = await mountLibraryScene(
    sceneHost,
    current.projection,
    reducedMotion.checked,
    (id) => {
      const book = allBooks(current.projection).find((candidate) => candidate.id === id);
      if (book) {
        void selectBook(book);
      }
    },
  );
  if (selectedId) {
    scene.select(selectedId);
  }
}

async function selectBook(book: LibraryBook): Promise<void> {
  if (viewer.dirty) {
    requireDirtyDecision("selecting another book");
    return;
  }
  selectedId = book.id;
  scene?.select(book.id);
  renderBookMeta(book);
  const requestedId = book.id;
  try {
    const resolution = activeResolution;
    if (!resolution) {
      throw new Error("the active library resolution is unavailable");
    }
    const document = await loadDocument(resolution.root, resolution.project, requestedId);
    if (selectedId === requestedId) {
      viewer.setDocument(document.source, isEditable(book));
    }
  } catch (error) {
    if (selectedId === requestedId) {
      viewer.setDocument("", false);
      setStatus(`Document unavailable: ${errorMessage(error)}`, "error");
    }
  }
}

function renderBookMeta(book: LibraryBook): void {
  const title = document.createElement("h3");
  title.textContent = book.label;
  const id = document.createElement("code");
  id.textContent = book.id;
  const explanation = document.createElement("p");
  explanation.textContent = book.explanation;
  const links = document.createElement("p");
  links.textContent = book.outgoing_links.length
    ? `Links: ${book.outgoing_links.join(", ")}`
    : "Links: none";
  const editability = document.createElement("p");
  editability.textContent = isEditable(book)
    ? "Editing: checked manual save"
    : "Editing: read-only for this note class or scope";
  metaHost.replaceChildren(title, id, explanation, links, editability);
}

async function saveSelectedNote(): Promise<void> {
  if (!library || !activeResolution || !selectedId || !viewer.editable || !viewer.dirty) {
    return;
  }
  saveButton.disabled = true;
  discardButton.disabled = true;
  setStatus("Saving through the checked Akasha Core transaction…");
  const id = selectedId;
  const resolution = activeResolution;
  try {
    const result = await saveDocument(
      resolution.root,
      resolution.project,
      id,
      viewer.savedSource,
      viewer.source,
    );
    viewer.markSaved();
    await openLibrary(id, resolution);
    setStatus(
      result.changed
        ? `Saved ${id}; project state validated.`
        : `${id} already matched the requested source.`,
      "success",
    );
  } catch (error) {
    setStatus(`Save failed: ${errorMessage(error)}`, "error");
    updateEditActions();
  }
}

function isEditable(book: LibraryBook): boolean {
  return (
    book.scope.kind === "project" &&
    book.scope.project === library?.projection.selected_project &&
    book.class !== "event"
  );
}

function updateEditActions(): void {
  saveButton.disabled = !viewer.editable || !viewer.dirty;
  discardButton.disabled = !viewer.dirty;
  editState.textContent = viewer.dirty ? "Unsaved changes" : "";
  editActions.classList.toggle("is-dirty", viewer.dirty);
  if (!viewer.dirty) {
    editActions.classList.remove("needs-decision");
  }
}

function requireDirtyDecision(action: string): void {
  setStatus(`Unsaved changes: choose Save or Discard before ${action}.`, "warning");
  editActions.classList.add("needs-decision");
  saveButton.focus();
}

type StatusTone = "neutral" | "success" | "warning" | "error";

function setStatus(message: string, tone: StatusTone = "neutral"): void {
  status.textContent = message;
  status.dataset.tone = tone;
  status.setAttribute("aria-live", tone === "warning" || tone === "error" ? "assertive" : "polite");
}

function errorMessage(error: unknown): string {
  if (typeof error === "object" && error !== null && "message" in error) {
    return String((error as CommandError).message);
  }
  return String(error);
}

function required<T extends HTMLElement>(id: string): T {
  const element = document.getElementById(id);
  if (!element) {
    throw new Error(`missing required element #${id}`);
  }
  return element as T;
}
