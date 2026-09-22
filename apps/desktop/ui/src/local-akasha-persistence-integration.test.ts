// @vitest-environment jsdom
import { expect, it, vi } from "vitest";
import shell from "../index.html?raw";
import type {
  DesktopLibrary,
  LibraryBook,
  LocalNavigationState,
} from "./types";

it("restores one revalidated local note without changing the global startup default", async () => {
  vi.resetModules();
  document.body.innerHTML = new DOMParser().parseFromString(shell, "text/html").body.innerHTML;
  history.replaceState(null, "", "/");
  vi.stubGlobal("matchMedia", () => ({
    matches: true,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    addListener: vi.fn(),
    removeListener: vi.fn(),
  }));
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
  Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
  Range.prototype.getBoundingClientRect = () => new DOMRect();

  const book: LibraryBook = {
    id: "Projects/example/entities/restored.md",
    label: "Restored",
    scope: { kind: "project", project: "example" },
    note_type: "entity",
    class: "entity",
    status: "active",
    reviewed: null,
    date: null,
    outgoing_links: [],
    explanation: "restored fixture",
  };
  const library: DesktopLibrary = {
    recovery: "none",
    fallback_markdown: "",
    projection: {
      root: "/resolved/root",
      selected_project: "example",
      total_books: 1,
      global: { categories: [] },
      projects: [{
        project: "example",
        status: "active",
        categories: [{ note_type: "entity", class: "entity", books: [book] }],
      }],
      dashboard: {
        validation_passed: true,
        projects: 1,
        notes: 1,
        global_notes: 0,
        configured_categories: 1,
        open_tasks: 0,
        open_problems: 0,
        validated_links: 0,
        latest_activity_date: null,
        project_metrics: [],
      },
    },
  };
  const persisted: LocalNavigationState = {
    version: 1,
    root: library.projection.root,
    project: library.projection.selected_project,
    section: "entity",
    sky_anchor: "entity",
    page_anchors: { entity: book.id },
    note: book.id,
  };
  const loadDocument = vi.fn(async () => ({ id: book.id, source: "# Restored fresh source\n" }));
  const saveLocalNavigation = vi.fn(async (_state: LocalNavigationState) => undefined);
  vi.doMock("./api", () => ({
    loadLibrary: vi.fn(async () => library),
    loadDocument,
    saveDocument: vi.fn(),
    searchProject: vi.fn(),
    loadLocalNavigation: vi.fn(async () => persisted),
    saveLocalNavigation,
  }));
  vi.doMock("./scene", () => ({
    mountLibraryScene: vi.fn(async () => ({
      aimedShelfId: () => null,
      select: vi.fn(),
      destroy: vi.fn(),
    })),
  }));

  await import("./main");
  await vi.waitFor(() => expect(document.querySelector(".brand-name")!.textContent).toBe("AKASHA LIBRARY"));
  expect(loadDocument).not.toHaveBeenCalled();

  document.querySelector<HTMLButtonElement>("#scope-toggle")!.click();
  await vi.waitFor(() => expect(loadDocument).toHaveBeenCalledWith(
    "/resolved/root", "example", book.id,
  ));
  await vi.waitFor(() => expect(document.querySelector<HTMLElement>("#note-overlay")!.hidden).toBe(false));
  expect(document.querySelector("#book-meta code")!.textContent).toBe(book.id);
  await vi.waitFor(() => expect(saveLocalNavigation).toHaveBeenCalled());
  const restoredSave = saveLocalNavigation.mock.calls.at(-1)![0];
  expect(restoredSave).toEqual(persisted);
  expect(JSON.stringify(restoredSave)).not.toMatch(/source|draft|query/u);

  document.querySelector<HTMLButtonElement>("#note-close")!.click();
  await vi.waitFor(() => expect(saveLocalNavigation.mock.calls.at(-1)![0].note).toBeNull());

  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});
