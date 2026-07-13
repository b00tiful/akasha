import "./styles.css";

import { loadDocument, loadLibrary } from "./api";
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
const viewer = new NoteViewer(required<HTMLElement>("note-viewer"));
const reducedMotion = required<HTMLInputElement>("reduced-motion");
const modeButtons = [...document.querySelectorAll<HTMLButtonElement>("[data-mode]")];

let library: DesktopLibrary | null = null;
let scene: SceneHandle | null = null;
let selectedId: string | null = null;

reducedMotion.checked = window.matchMedia("(prefers-reduced-motion: reduce)").matches;

form.addEventListener("submit", (event) => {
  event.preventDefault();
  void openLibrary();
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

async function openLibrary(): Promise<void> {
  status.textContent = "Validating the synthetic library through Akasha Core…";
  form.classList.add("is-loading");
  try {
    library = await loadLibrary(rootInput.value, projectInput.value);
    selectedId = null;
    renderFallback(fallbackHost, library.projection, (book) => void selectBook(book));
    await renderScene(library);
    const first = allBooks(library.projection)[0];
    if (first) {
      await selectBook(first);
    }
    status.textContent = `${library.projection.total_books} canonical books / ${library.projection.projects.length} project shelves / validation passed`;
  } catch (error) {
    library = null;
    scene?.destroy();
    scene = null;
    sceneHost.replaceChildren();
    fallbackHost.replaceChildren();
    metaHost.textContent = errorMessage(error);
    viewer.setDocument("");
    status.textContent = `Library unavailable: ${errorMessage(error)}`;
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
  selectedId = book.id;
  scene?.select(book.id);
  renderBookMeta(book);
  const requestedId = book.id;
  try {
    const document = await loadDocument(rootInput.value, projectInput.value, requestedId);
    if (selectedId === requestedId) {
      viewer.setDocument(document.source);
    }
  } catch (error) {
    if (selectedId === requestedId) {
      viewer.setDocument("");
      status.textContent = `Document unavailable: ${errorMessage(error)}`;
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
  metaHost.replaceChildren(title, id, explanation, links);
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
