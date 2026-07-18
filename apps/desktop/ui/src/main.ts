import "./styles.css";

import { loadDocument, loadLibrary, saveDocument } from "./api";
import { NoteViewer, type ViewerMode } from "./editor";
import { renderFallback } from "./fallback";
import {
  allBooks,
  libraryShelves,
  volumeForBook,
  volumesForShelf,
  type LibraryVolume,
  type VisualShelf,
} from "./projection";
import { mountLibraryScene, type SceneHandle } from "./scene";
import type { SpatialDirection } from "./scene-model";
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
const noteOverlay = required<HTMLElement>("note-overlay");
const noteClose = required<HTMLButtonElement>("note-close");
const previousShelf = required<HTMLButtonElement>("previous-shelf");
const nextShelf = required<HTMLButtonElement>("next-shelf");
const activeShelfLabel = required<HTMLElement>("active-shelf-label");
const selectionShelf = required<HTMLElement>("selection-shelf");
const selectionCount = required<HTMLElement>("selection-count");
const selectionVolume = required<HTMLElement>("selection-volume");
const volumePanel = required<HTMLElement>("volume-panel");
const volumeTitle = required<HTMLElement>("volume-title");
const volumeMeta = required<HTMLElement>("volume-meta");
const volumeBooks = required<HTMLElement>("volume-books");
const volumeClose = required<HTMLButtonElement>("volume-close");
const dashboard = required<HTMLElement>("dashboard");
const dashboardToggle = required<HTMLButtonElement>("dashboard-toggle");
const dashboardExpand = required<HTMLButtonElement>("dashboard-expand");
const dashboardCompact = required<HTMLElement>("dashboard-compact");
const dashboardSummary = required<HTMLElement>("dashboard-summary");
const dashboardProjects = required<HTMLElement>("dashboard-projects");
const inventoryPanel = required<HTMLElement>("inventory-panel");
const inventoryToggle = required<HTMLButtonElement>("inventory-toggle");
const inventoryClose = required<HTMLButtonElement>("inventory-close");
const settingsPanel = required<HTMLElement>("settings-panel");
const settingsToggle = required<HTMLButtonElement>("settings-toggle");
const settingsClose = required<HTMLButtonElement>("settings-close");
const reducedMotion = required<HTMLInputElement>("reduced-motion");
const modeButtons = [...document.querySelectorAll<HTMLButtonElement>("[data-mode]")];

let viewer: NoteViewer;
viewer = new NoteViewer(required<HTMLElement>("note-viewer"), updateEditActions);

let library: DesktopLibrary | null = null;
let scene: SceneHandle | null = null;
let selectedId: string | null = null;
let aimedShelfId: string | null = null;
let activeShelfId: string | null = null;
let activeVolume: LibraryVolume | null = null;
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

previousShelf.addEventListener("click", () => moveShelfAim("left"));
nextShelf.addEventListener("click", () => moveShelfAim("right"));
volumeClose.addEventListener("click", () => void closeVolume());
noteClose.addEventListener("click", closeNote);
dashboardToggle.addEventListener("click", toggleDashboard);
dashboardExpand.addEventListener("click", toggleDashboard);
inventoryToggle.addEventListener("click", () => toggleDrawer(inventoryPanel, inventoryToggle));
inventoryClose.addEventListener("click", () => closeDrawer(inventoryPanel, inventoryToggle));
settingsToggle.addEventListener("click", () => toggleDrawer(settingsPanel, settingsToggle));
settingsClose.addEventListener("click", () => closeDrawer(settingsPanel, settingsToggle));

saveButton.addEventListener("click", () => void saveSelectedNote());
discardButton.addEventListener("click", () => {
  viewer.discard();
  setStatus("Changes discarded. You can navigate away.", "success");
});

reducedMotion.addEventListener("change", () => {
  scene?.setReducedMotion(reducedMotion.checked);
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

window.addEventListener("keydown", (event) => {
  if (isTypingTarget(event.target)) {
    return;
  }
  if (event.key === "Tab") {
    event.preventDefault();
    toggleDrawer(inventoryPanel, inventoryToggle);
    return;
  }
  if (event.key === "Escape") {
    event.preventDefault();
    closeTopLayer();
    return;
  }
  if (!noteOverlay.hidden || !inventoryPanel.hidden || !settingsPanel.hidden) {
    return;
  }
  const key = event.key.toLowerCase();
  if (!activeShelfId) {
    const direction = spatialDirection(key, event.key);
    if (direction) {
      event.preventDefault();
      moveShelfAim(direction);
      return;
    }
  }
  if (event.key === "Enter" || event.key === " ") {
    const shelf = currentShelf();
    if (!activeShelfId && shelf) {
      event.preventDefault();
      void chooseShelf(shelf.id);
      return;
    }
    const firstVolume = shelf ? volumesForShelf(shelf)[0] : undefined;
    if (firstVolume) {
      event.preventDefault();
      void openVolume(firstVolume);
    }
  }
});

void openLibrary();

async function openLibrary(
  preferredId?: string,
  requestedResolution = { root: rootInput.value, project: projectInput.value },
): Promise<void> {
  setStatus("Validating the library through Akasha Core…");
  form.classList.add("is-loading");
  try {
    library = await loadLibrary(requestedResolution.root, requestedResolution.project);
    activeResolution = requestedResolution;
    rootInput.value = requestedResolution.root;
    projectInput.value = requestedResolution.project;
    selectedId = null;
    aimedShelfId = null;
    activeShelfId = null;
    activeVolume = null;

    const shelves = libraryShelves(library.projection);
    const preferred = preferredId
      ? allBooks(library.projection).find((book) => book.id === preferredId)
      : undefined;
    const preferredShelf = preferred
      ? shelves.find((shelf) => volumeForBook(shelf, preferred.id) !== undefined)
      : undefined;
    aimedShelfId = preferredShelf?.id ?? null;
    activeShelfId = preferredShelf?.id ?? null;
    activeVolume = preferred && preferredShelf ? volumeForBook(preferredShelf, preferred.id) ?? null : null;

    renderFallback(fallbackHost, library.projection, (book) => void openBookFromInventory(book));
    renderDashboard(library);
    renderVolume();
    await renderScene(library);
    renderSelection();
    closeDrawer(settingsPanel, settingsToggle);

    if (preferred) {
      await selectBook(preferred);
    } else {
      noteOverlay.hidden = true;
      viewer.setDocument("", false);
      metaHost.innerHTML = "<p>Select a note from an open volume.</p>";
    }

    const recovery = library.recovery === "none" ? "" : ` / recovery ${library.recovery}`;
    setStatus(
      `${library.projection.total_books} canonical notes / ${library.projection.projects.length} projects / validation passed${recovery}`,
      "success",
    );
  } catch (error) {
    library = null;
    activeResolution = null;
    aimedShelfId = null;
    activeShelfId = null;
    activeVolume = null;
    scene?.destroy();
    scene = null;
    sceneHost.replaceChildren();
    fallbackHost.replaceChildren();
    noteOverlay.hidden = true;
    volumePanel.hidden = true;
    metaHost.textContent = errorMessage(error);
    viewer.setDocument("", false);
    renderSelection();
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
    aimedShelfId,
    activeShelfId,
    activeVolume?.id ?? null,
    reducedMotion.checked,
    {
      onAimShelf: (id) => {
        aimedShelfId = id;
        renderSelection();
      },
      onSelectShelf: (id) => void chooseShelf(id),
      onSelectVolume: (volume) => void openVolume(volume),
    },
  );
  aimedShelfId = scene.aimedShelfId();
  scene.select(selectedId);
}

async function chooseShelf(id: string): Promise<void> {
  if (!library || !libraryShelves(library.projection).some((shelf) => shelf.id === id)) {
    return;
  }
  if (id === activeShelfId) {
    return;
  }
  if (viewer.dirty) {
    requireDirtyDecision("changing shelves");
    return;
  }
  selectedId = null;
  aimedShelfId = id;
  activeShelfId = id;
  activeVolume = null;
  noteOverlay.hidden = true;
  volumePanel.hidden = true;
  viewer.setDocument("", false);
  scene?.aimShelf(id);
  scene?.activateShelf(id);
  scene?.openVolume(null);
  scene?.select(null);
  renderSelection();
  setStatus(`${currentShelf()?.label ?? "Shelf"} brought into focus.`, "success");
}

function moveShelfAim(direction: SpatialDirection): void {
  if (!scene || activeShelfId) {
    return;
  }
  aimedShelfId = scene.moveAim(direction);
  renderSelection();
  const shelf = currentShelf();
  if (shelf) {
    setStatus(`${shelf.label} aimed. Press Enter or Space to activate.`, "success");
  }
}

async function openVolume(volume: LibraryVolume): Promise<void> {
  if (!library) {
    return;
  }
  if (viewer.dirty) {
    requireDirtyDecision("opening another volume");
    return;
  }
  selectedId = null;
  aimedShelfId = volume.shelfId;
  activeShelfId = volume.shelfId;
  activeVolume = volume;
  noteOverlay.hidden = true;
  viewer.setDocument("", false);
  scene?.aimShelf(volume.shelfId);
  scene?.activateShelf(volume.shelfId);
  scene?.openVolume(volume.id);
  scene?.select(null);
  renderSelection();
  renderVolume();
  setStatus(`${volume.noteType} ${volume.label} opened.`, "success");
}

async function closeVolume(): Promise<void> {
  if (!library) {
    return;
  }
  if (viewer.dirty) {
    requireDirtyDecision("closing the volume");
    return;
  }
  activeVolume = null;
  selectedId = null;
  noteOverlay.hidden = true;
  volumePanel.hidden = true;
  viewer.setDocument("", false);
  scene?.openVolume(null);
  scene?.select(null);
  renderSelection();
}

async function openBookFromInventory(book: LibraryBook): Promise<void> {
  if (!library) {
    return;
  }
  if (viewer.dirty) {
    requireDirtyDecision("selecting another note");
    return;
  }
  const shelf = libraryShelves(library.projection).find(
    (candidate) => volumeForBook(candidate, book.id) !== undefined,
  );
  if (!shelf) {
    setStatus(`No projected shelf contains ${book.id}.`, "error");
    return;
  }
  activeShelfId = shelf.id;
  aimedShelfId = shelf.id;
  activeVolume = volumeForBook(shelf, book.id) ?? null;
  closeDrawer(inventoryPanel, inventoryToggle);
  scene?.aimShelf(shelf.id);
  scene?.activateShelf(shelf.id);
  scene?.openVolume(activeVolume?.id ?? null);
  renderSelection();
  renderVolume();
  await selectBook(book);
}

async function selectBook(book: LibraryBook): Promise<void> {
  if (viewer.dirty) {
    requireDirtyDecision("selecting another note");
    return;
  }
  selectedId = book.id;
  scene?.select(book.id);
  renderBookMeta(book);
  noteOverlay.hidden = false;
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

function closeNote(): void {
  if (viewer.dirty) {
    requireDirtyDecision("closing the note");
    return;
  }
  noteOverlay.hidden = true;
  selectedId = null;
  scene?.select(null);
  viewer.setDocument("", false);
  metaHost.innerHTML = "<p>Select a note from an open volume.</p>";
}

function renderSelection(): void {
  const shelf = currentShelf();
  activeShelfLabel.textContent = shelf?.label ?? "Archive unavailable";
  selectionShelf.textContent = shelf?.label ?? "No shelf aimed";
  selectionCount.textContent = `${shelf?.noteCount ?? 0} notes`;
  selectionVolume.textContent = activeVolume
    ? `${activeVolume.noteType} / ${activeVolume.label}`
    : activeShelfId
      ? "Choose a volume"
      : "Press Enter or Space to activate";
}

function renderVolume(): void {
  volumeBooks.replaceChildren();
  if (!activeVolume) {
    volumePanel.hidden = true;
    return;
  }
  volumePanel.hidden = false;
  volumeTitle.textContent = `${activeVolume.noteType} ${activeVolume.label}`;
  volumeMeta.textContent = `${activeVolume.books.length} notes · volume ${activeVolume.index} · maximum 20`;
  for (const book of activeVolume.books) {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "volume-book";
    button.textContent = book.label;
    button.title = `${book.id}\n${book.explanation}`;
    button.addEventListener("click", () => void selectBook(book));
    volumeBooks.append(button);
  }
}

function renderDashboard(current: DesktopLibrary): void {
  const metrics = current.projection.dashboard;
  dashboardCompact.replaceChildren(
    metric("Projects", metrics.projects),
    metric("Notes", metrics.notes),
    metric("Open tasks", metrics.open_tasks, metrics.open_tasks > 0),
    metric("Problems", metrics.open_problems, metrics.open_problems > 0),
    metric("Links", metrics.validated_links),
    metric("Validated", metrics.validation_passed ? "yes" : "no", !metrics.validation_passed),
  );

  dashboardSummary.replaceChildren(
    summaryCard("All notes", metrics.notes),
    summaryCard("Global notes", metrics.global_notes),
    summaryCard("Categories", metrics.configured_categories),
    summaryCard("Latest activity", metrics.latest_activity_date ?? "none"),
  );

  dashboardProjects.replaceChildren();
  for (const project of metrics.project_metrics) {
    const row = document.createElement("tr");
    for (const value of [
      project.project,
      project.status,
      project.notes,
      project.open_tasks,
      project.open_problems,
      project.validated_links,
      project.latest_activity_date ?? "—",
    ]) {
      const cell = document.createElement("td");
      cell.textContent = String(value);
      row.append(cell);
    }
    dashboardProjects.append(row);
  }
}

function metric(label: string, value: string | number, warning = false): HTMLElement {
  const item = document.createElement("div");
  item.className = warning ? "metric is-warning" : "metric";
  const name = document.createElement("span");
  name.textContent = label;
  const result = document.createElement("strong");
  result.textContent = String(value);
  item.append(name, result);
  return item;
}

function summaryCard(label: string, value: string | number): HTMLElement {
  const card = document.createElement("div");
  card.className = "summary-card";
  const name = document.createElement("span");
  name.textContent = label;
  const result = document.createElement("strong");
  result.textContent = String(value);
  card.append(name, result);
  return card;
}

function toggleDashboard(): void {
  const expanded = dashboard.classList.toggle("is-expanded");
  dashboardExpand.textContent = expanded ? "Collapse" : "Expand";
  dashboardExpand.setAttribute("aria-expanded", String(expanded));
  dashboardToggle.setAttribute("aria-pressed", String(expanded));
}

function toggleDrawer(panel: HTMLElement, trigger: HTMLButtonElement): void {
  const willOpen = panel.hidden;
  closeDrawer(inventoryPanel, inventoryToggle);
  closeDrawer(settingsPanel, settingsToggle);
  panel.hidden = !willOpen;
  trigger.setAttribute("aria-expanded", String(willOpen));
}

function closeDrawer(panel: HTMLElement, trigger: HTMLButtonElement): void {
  panel.hidden = true;
  trigger.setAttribute("aria-expanded", "false");
}

function closeTopLayer(): void {
  if (!noteOverlay.hidden) {
    closeNote();
  } else if (!volumePanel.hidden) {
    void closeVolume();
  } else if (!inventoryPanel.hidden) {
    closeDrawer(inventoryPanel, inventoryToggle);
  } else if (!settingsPanel.hidden) {
    closeDrawer(settingsPanel, settingsToggle);
  } else if (dashboard.classList.contains("is-expanded")) {
    toggleDashboard();
  } else if (activeShelfId) {
    closeShelf();
  }
}

function currentShelf(): VisualShelf | undefined {
  if (!library) {
    return undefined;
  }
  const currentId = activeShelfId ?? aimedShelfId;
  return libraryShelves(library.projection).find((shelf) => shelf.id === currentId);
}

function closeShelf(): void {
  if (!activeShelfId) {
    return;
  }
  activeVolume = null;
  activeShelfId = null;
  selectedId = null;
  noteOverlay.hidden = true;
  volumePanel.hidden = true;
  viewer.setDocument("", false);
  scene?.openVolume(null);
  scene?.select(null);
  scene?.deactivateShelf();
  renderSelection();
  setStatus(`${currentShelf()?.label ?? "Shelf"} returned to the library.`, "success");
}

function spatialDirection(key: string, originalKey: string): SpatialDirection | null {
  if (key === "w" || originalKey === "ArrowUp") {
    return "up";
  }
  if (key === "s" || originalKey === "ArrowDown") {
    return "down";
  }
  if (key === "a" || originalKey === "ArrowLeft") {
    return "left";
  }
  if (key === "d" || originalKey === "ArrowRight") {
    return "right";
  }
  return null;
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

function isTypingTarget(target: EventTarget | null): boolean {
  return target instanceof HTMLElement &&
    (target.matches("input, textarea, [contenteditable='true']") || target.closest(".cm-editor") !== null);
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
