import "./styles.css";

import { loadDocument, loadLibrary, saveDocument, searchProject } from "./api";
import { NoteViewer, type ViewerMode } from "./editor";
import { renderFallback, renderLocalFallback } from "./fallback";
import { localAkashaModel } from "./local-akasha-model";
import { mountLocalAkasha, type LocalNavigation, type LocalSceneHandle } from "./local-akasha-scene";
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
import type { CommandError, DesktopLibrary, LibraryBook, LibrarySearchResult } from "./types";

const form = required<HTMLFormElement>("library-form");
const rootInput = required<HTMLInputElement>("root-input");
const projectInput = required<HTMLInputElement>("project-input");
const status = required<HTMLElement>("status");
const sceneHost = required<HTMLElement>("scene");
const scopeToggle = required<HTMLButtonElement>("scope-toggle");
const stage = document.querySelector<HTMLElement>(".pixel-stage")!;
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
const dashboardActionLabel = required<HTMLElement>("dashboard-action-label");
const dashboardCompact = required<HTMLElement>("dashboard-compact");
const dashboardSigil = required<HTMLElement>("dashboard-sigil");
const dashboardSigilProjects = required<HTMLElement>("dashboard-sigil-projects");
const dashboardSigilCategories = required<HTMLElement>("dashboard-sigil-categories");
const dashboardSummary = required<HTMLElement>("dashboard-summary");
const dashboardProjects = required<HTMLElement>("dashboard-projects");
const inventoryPanel = required<HTMLElement>("inventory-panel");
const inventoryToggle = required<HTMLButtonElement>("inventory-toggle");
const inventoryClose = required<HTMLButtonElement>("inventory-close");
const searchPanel = required<HTMLElement>("search-panel");
const searchToggle = required<HTMLButtonElement>("search-toggle");
const searchClose = required<HTMLButtonElement>("search-close");
const searchForm = required<HTMLFormElement>("search-form");
const searchInput = required<HTMLInputElement>("search-input");
const searchSummary = required<HTMLElement>("search-summary");
const searchResults = required<HTMLElement>("search-results");
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
let localScene: LocalSceneHandle | null = null;
let localMode = new URLSearchParams(location.search).get("view") === "local";
const localNavigation = new Map<string, LocalNavigation>();
let documentRequest = 0;
let searchRequest = 0;
let switchingScope = false;

reducedMotion.checked = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
document.documentElement.classList.toggle("reduced-motion", reducedMotion.checked);

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
searchToggle.addEventListener("click", toggleSearch);
searchClose.addEventListener("click", closeSearch);
searchForm.addEventListener("submit", (event) => {
  event.preventDefault();
  void runProjectSearch();
});
settingsToggle.addEventListener("click", () => toggleDrawer(settingsPanel, settingsToggle));
settingsClose.addEventListener("click", () => closeDrawer(settingsPanel, settingsToggle));

saveButton.addEventListener("click", () => void saveSelectedNote());
discardButton.addEventListener("click", () => {
  viewer.discard();
  setStatus("Changes discarded. You can navigate away.", "success");
});

reducedMotion.addEventListener("change", () => {
  document.documentElement.classList.toggle("reduced-motion", reducedMotion.checked);
  scene?.setReducedMotion(reducedMotion.checked);
  localScene?.setReducedMotion(reducedMotion.checked);
});

window.matchMedia("(prefers-reduced-motion: reduce)").addEventListener("change", (event) => {
  reducedMotion.checked = event.matches;
  reducedMotion.dispatchEvent(new Event("change"));
});
scopeToggle.addEventListener("click", () => void switchScope());

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
  if (localMode) {
    if (event.key === "Tab" && !noteOverlay.hidden) {
      const controls = [...noteOverlay.querySelectorAll<HTMLElement>('button:not(:disabled), [tabindex="0"], .cm-content')]
        .filter((node) => node.getClientRects().length > 0);
      const first = controls[0];
      const last = controls.at(-1);
      if (first && last && ((event.shiftKey && document.activeElement === first) ||
        (!event.shiftKey && document.activeElement === last))) {
        event.preventDefault();
        (event.shiftKey ? last : first).focus();
      }
    }
    if (event.key === "Escape") {
      event.preventDefault();
      closeTopLayer();
    }
    return;
  }
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
  documentRequest++;
  clearSearch();
  form.classList.add("is-loading");
  try {
    library = await loadLibrary(requestedResolution.root, requestedResolution.project);
    activeResolution = { root: library.projection.root, project: library.projection.selected_project };
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
    closeSearch();

    if (preferred) {
      await selectBook(preferred);
    } else {
      noteOverlay.hidden = true;
      viewer.setDocument("", false);
      metaHost.innerHTML = "<p>Select a note from an open volume.</p>";
    }

    const recovery = library.recovery === "none" ? "" : ` / recovery ${library.recovery}`;
    setStatus(
      localMode
        ? `Local project ${library.projection.selected_project} / validation passed${recovery}`
        : `${library.projection.total_books} canonical notes / ${library.projection.projects.length} projects / validation passed${recovery}`,
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
    localScene?.destroy();
    localScene = null;
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
  scene = null;
  localScene?.destroy();
  localScene = null;
  stage.classList.toggle("is-local", localMode);
  noteOverlay.setAttribute("role", localMode ? "dialog" : "region");
  if (localMode) noteOverlay.setAttribute("aria-modal", "true");
  else noteOverlay.removeAttribute("aria-modal");
  stage.querySelector(".brand-name")!.textContent = localMode ? "AKASHA LOCAL VAULT" : "AKASHA LIBRARY";
  stage.querySelector(".brand-plaque p:last-child")!.textContent = localMode ? current.projection.selected_project : "global archive";
  renderDashboard(current);
  stage.setAttribute("aria-label", localMode ? "Akasha local vault" : "Akasha global library");
  scopeToggle.textContent = localMode ? "Global library" : "Local Akasha";
  sceneHost.setAttribute("aria-label", localMode ? "Selected project directory sky" : "Floating three-dimensional project bookshelves");
  (localMode ? renderLocalFallback : renderFallback)(fallbackHost, current.projection, (book) => void openBookFromInventory(book));
  if (localMode) {
    const model = localAkashaModel(current.projection);
    let navigation = localNavigation.get(model.key);
    if (!navigation) {
      navigation = { section: null, skyPage: 0, pages: {} };
      localNavigation.set(model.key, navigation);
    }
    localScene = mountLocalAkasha(sceneHost, model, navigation, reducedMotion.checked, {
      canNavigate: () => {
        if (viewer.dirty) { requireDirtyDecision("leaving this note"); return false; }
        closeNote();
        return true;
      },
      onSelect: (book) => void selectBook(book),
      onBack: () => { if (!noteOverlay.hidden) closeNote(); else localScene?.back(); },
    });
    volumePanel.hidden = true;
    return;
  }
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

async function switchScope(): Promise<void> {
  if (!library || switchingScope) return;
  if (viewer.dirty) { requireDirtyDecision("switching between local and global views"); return; }
  switchingScope = true;
  scopeToggle.disabled = true;
  closeNote();
  closeDrawer(inventoryPanel, inventoryToggle);
  closeSearch();
  closeDrawer(settingsPanel, settingsToggle);
  localMode = !localMode;
  try {
    await renderScene(library);
    if (!localMode) renderVolume();
    setStatus(localMode ? `Local project ${library.projection.selected_project}` : "Global archive", "success");
    scopeToggle.focus();
  } catch (error) {
    setStatus(`View unavailable: ${errorMessage(error)}`, "error");
  } finally {
    switchingScope = false;
    scopeToggle.disabled = false;
  }
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
  if (localMode) {
    if (book.scope.kind !== "project" || book.scope.project !== library.projection.selected_project) {
      setStatus("This note belongs to the global archive. Switch to Global library to open it.", "warning");
      return;
    }
    closeDrawer(inventoryPanel, inventoryToggle);
    closeSearch();
    await selectBook(book);
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
  const request = ++documentRequest;
  viewer.setDocument("", false);
  scene?.select(book.id);
  localScene?.select(book.id);
  renderBookMeta(book);
  noteOverlay.hidden = false;
  const requestedId = book.id;
  try {
    const resolution = activeResolution;
    if (!resolution) {
      throw new Error("the active library resolution is unavailable");
    }
    const document = await loadDocument(resolution.root, resolution.project, requestedId);
    if (selectedId === requestedId && request === documentRequest) {
      viewer.setDocument(document.source, isEditable(book));
      if (localMode) noteClose.focus();
    }
  } catch (error) {
    if (selectedId === requestedId && request === documentRequest) {
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
  documentRequest++;
  selectedId = null;
  scene?.select(null);
  localScene?.select(null);
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
  const local = localMode ? localAkashaModel(current.projection) : null;
  const selectedMetrics = metrics.project_metrics.find((project) => project.project === current.projection.selected_project);
  required<HTMLElement>("dashboard-title").textContent = local ? "Vault status" : "Library status";
  dashboard.querySelector<HTMLElement>(".dashboard-ledger-title")!.textContent = local ? "LOCAL SNAPSHOT" : "VALIDATED SNAPSHOT";
  dashboard.querySelector<HTMLElement>(".dashboard-footnote")!.textContent = local
    ? `Selected project: ${local.project}` : "Timeline chamber — sealed for a later slice.";
  dashboardSigilProjects.textContent = `P ${metrics.projects}`;
  dashboardSigilProjects.title = `${metrics.projects} projects`;
  dashboardSigilCategories.textContent = `C ${metrics.configured_categories}`;
  dashboardSigilCategories.title = `${metrics.configured_categories} configured categories`;
  dashboardSigil.dataset.validation = metrics.validation_passed ? "passed" : "failed";
  dashboardSigil.setAttribute(
    "aria-label",
    `Archive map: ${metrics.projects} projects and ${metrics.configured_categories} configured categories orbit the ${metrics.validation_passed ? "validated" : "unvalidated"} library core.`,
  );
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
    if (local && project.project !== local.project) continue;
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
  if (local) {
    dashboardSigilProjects.textContent = `N ${local.noteCount}`;
    dashboardSigilProjects.title = `${local.noteCount} local notes`;
    dashboardSigilCategories.textContent = `S ${local.sections.length}`;
    dashboardSigilCategories.title = `${local.sections.length} local sections`;
    dashboardSigil.setAttribute("aria-label", `Local vault: ${local.project}, ${local.noteCount} notes and ${local.sections.length} sections; validation ${metrics.validation_passed ? "passed" : "failed"}.`);
    dashboardCompact.replaceChildren(
      metric("Sections", local.sections.length), metric("Notes", local.noteCount),
      metric("Open tasks", selectedMetrics?.open_tasks ?? "—", (selectedMetrics?.open_tasks ?? 0) > 0),
      metric("Problems", selectedMetrics?.open_problems ?? "—", (selectedMetrics?.open_problems ?? 0) > 0),
      metric("Links", selectedMetrics?.validated_links ?? "—"),
      metric("Validated", metrics.validation_passed ? "yes" : "no", !metrics.validation_passed),
    );
    dashboardSummary.replaceChildren(
      summaryCard("Project", local.project), summaryCard("Local notes", local.noteCount),
      summaryCard("Populated sections", selectedMetrics?.populated_categories ?? "—"),
      summaryCard("Latest activity", selectedMetrics ? selectedMetrics.latest_activity_date ?? "none" : "—"),
    );
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
  dashboardActionLabel.textContent = expanded ? "Collapse" : "Expand";
  dashboardExpand.setAttribute("aria-expanded", String(expanded));
  dashboardToggle.setAttribute("aria-pressed", String(expanded));
  dashboardToggle.setAttribute("aria-expanded", String(expanded));
}

function toggleDrawer(panel: HTMLElement, trigger: HTMLButtonElement): void {
  const willOpen = panel.hidden;
  closeDrawer(inventoryPanel, inventoryToggle);
  closeSearch();
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
  } else if (!searchPanel.hidden) {
    closeSearch();
  } else if (!settingsPanel.hidden) {
    closeDrawer(settingsPanel, settingsToggle);
  } else if (dashboard.classList.contains("is-expanded")) {
    toggleDashboard();
  } else if (localMode) {
    localScene?.back();
  } else if (activeShelfId) {
    closeShelf();
  }
}

function toggleSearch(): void {
  if (!localMode || !library) return;
  const willOpen = searchPanel.hidden;
  closeDrawer(inventoryPanel, inventoryToggle);
  closeSearch();
  closeDrawer(settingsPanel, settingsToggle);
  searchPanel.hidden = !willOpen;
  searchToggle.setAttribute("aria-expanded", String(willOpen));
  if (willOpen) searchInput.focus();
}

function closeSearch(): void {
  searchPanel.hidden = true;
  searchToggle.setAttribute("aria-expanded", "false");
}

function clearSearch(): void {
  searchRequest++;
  searchInput.value = "";
  searchSummary.textContent = "Enter 1–256 characters.";
  searchResults.replaceChildren();
  searchForm.removeAttribute("aria-busy");
}

async function runProjectSearch(): Promise<void> {
  if (!localMode || !library || !activeResolution) return;
  const request = ++searchRequest;
  const resolution = activeResolution;
  searchForm.setAttribute("aria-busy", "true");
  searchSummary.textContent = `Searching ${resolution.project}…`;
  searchResults.replaceChildren();
  try {
    const result = await searchProject(resolution.root, resolution.project, searchInput.value);
    if (request !== searchRequest || resolution !== activeResolution || !localMode) return;
    renderSearchResult(result);
  } catch (error) {
    if (request !== searchRequest) return;
    searchSummary.textContent = `Search failed: ${errorMessage(error)}`;
    setStatus(`Search failed: ${errorMessage(error)}`, "error");
  } finally {
    if (request === searchRequest) searchForm.removeAttribute("aria-busy");
  }
}

function renderSearchResult(result: LibrarySearchResult): void {
  if (!library) return;
  const project = library.projection.selected_project;
  searchResults.replaceChildren();
  searchSummary.textContent = result.truncated
    ? `Showing ${result.hits.length} of ${result.total_matches} matches in ${project}.`
    : `${result.total_matches} ${result.total_matches === 1 ? "match" : "matches"} in ${project}.`;
  const books = new Map(
    library.projection.projects
      .find((shelf) => shelf.project === project)
      ?.categories.flatMap((category) => category.books)
      .map((book) => [book.id, book]) ?? [],
  );
  for (const hit of result.hits) {
    if (hit.scope.kind !== "project" || hit.scope.project !== project) continue;
    const book = books.get(hit.id);
    if (!book) continue;
    const node = document.createElement("button");
    node.type = "button";
    node.className = "search-hit";
    const title = document.createElement("strong");
    title.textContent = hit.label;
    const id = document.createElement("span");
    id.textContent = hit.line === null ? hit.id : `${hit.id}:${hit.line}`;
    const snippet = document.createElement("small");
    snippet.textContent = hit.snippet;
    node.append(title, id, snippet);
    node.addEventListener("click", () => void openBookFromInventory(book));
    searchResults.append(node);
  }
  if (result.total_matches === 0) {
    const empty = document.createElement("p");
    empty.className = "search-empty";
    empty.textContent = `No exact literal matches for “${result.query}”.`;
    searchResults.append(empty);
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
