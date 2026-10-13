// @vitest-environment jsdom
import { expect, it, vi } from "vitest";
import shell from "../index.html?raw";
import type { DesktopLibrary, LibraryBook, LinkedLibraryDocument, NoteLinksResult } from "./types";

it("keeps the shared viewer through backlink refusal, dirty changes, obsolete replies and Local scope", async () => {
  document.body.innerHTML = new DOMParser().parseFromString(shell, "text/html").body.innerHTML;
  history.replaceState(null, "", "/");
  vi.stubGlobal("matchMedia", () => ({ matches: true, addListener: vi.fn(), removeListener: vi.fn(), addEventListener: vi.fn(), removeEventListener: vi.fn() }));
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
  Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
  Range.prototype.getBoundingClientRect = () => new DOMRect();
  const books: LibraryBook[] = ["first", "second", "global"].map((name) => ({
    id: name === "global" ? "Global/entities/shared.md" : `Projects/example/entities/${name}.md`,
    label: name, scope: name === "global" ? { kind: "global" } : { kind: "project", project: "example" },
    note_type: "entity", class: "entity", status: "active", reviewed: null, date: null, outgoing_links: [], explanation: name,
  }));
  const [first, second, global] = books as [LibraryBook, LibraryBook, LibraryBook];
  const source = "# 🌘 First\r\nExact source.\r\n";
  const documents = new Map<string, LinkedLibraryDocument>([
    [first.id, { id: first.id, source, links: [] }],
    [second.id, { id: second.id, source: "# Second\n", links: [] }],
    [global.id, { id: global.id, source: "# Global\n", links: [] }],
  ]);
  const data: DesktopLibrary = { recovery: "none", fallback_markdown: "", projection: {
    root: "/resolved/root", selected_project: "example", total_books: 3,
    global: { categories: [{ note_type: "entity", class: "entity", books: [global] }] },
    projects: [{ project: "example", status: "active", categories: [{ note_type: "entity", class: "entity", books: [first, second] }] }],
    dashboard: { validation_passed: true, projects: 1, notes: 3, global_notes: 1, configured_categories: 1,
      open_tasks: 0, open_problems: 0, validated_links: 0, latest_activity_date: null,
      project_metrics: [{ project: "example", status: "active", notes: 2, populated_categories: 1,
        open_tasks: 0, open_problems: 0, validated_links: 0, latest_activity_date: null }] },
  } };
  const inspectDocumentBacklinks = vi.fn(async (_root: string, _project: string, id: string): Promise<NoteLinksResult> => ({
    note: id, direction: "incoming", scanned_canonical_notes: 3, scanned_projections: 2,
    total_matches: 3, truncated: false,
    links: [{ id: second.id, kind: "canonical" }, { id: global.id, kind: "canonical" }, { id: "Projects/example/index.md", kind: "projection" }],
  }));
  const pending: Array<{ resolve(value: LinkedLibraryDocument): void; reject(error: unknown): void }> = [];
  const followDocumentBacklink = vi.fn(() => new Promise<LinkedLibraryDocument>((resolve, reject) => pending.push({ resolve, reject })));
  const loadDocumentWithRecovery = vi.fn(async (_root: string, _project: string, id: string) => documents.get(id)!);
  vi.doMock("./api", () => ({ inspectDocumentBacklinks, followDocumentBacklink, loadDocumentWithRecovery,
    followDocumentLink: vi.fn(), loadLibraryWithRecovery: vi.fn(async () => data),
    loadLocalNavigation: vi.fn(async () => null), saveLocalNavigation: vi.fn(async () => undefined),
    inspectRecovery: vi.fn(), saveDocument: vi.fn(), searchProject: vi.fn(),
  }));
  vi.doMock("./scene", () => ({ mountLibraryScene: vi.fn(async () => ({ aimedShelfId: () => null,
    select: vi.fn(), destroy: vi.fn(), aimShelf: vi.fn(), activateShelf: vi.fn(), openVolume: vi.fn() })) }));
  const { NoteViewer } = await import("./editor");
  const setDocument = vi.spyOn(NoteViewer.prototype, "setDocument");
  await import("./main");
  await vi.waitFor(() => expect(document.querySelectorAll(".book-button")).toHaveLength(3));
  const open = async (id: string) => {
    [...document.querySelectorAll<HTMLButtonElement>(".book-button")].find((button) => button.textContent === id)!.click();
    await vi.waitFor(() => expect(setDocument).toHaveBeenLastCalledWith(documents.get(id)!.source, id !== global.id));
  };
  const panel = document.querySelector<HTMLDetailsElement>("#backlinks")!;
  const status = () => document.querySelector("#status")!.textContent;
  const button = (index = 0) => panel.querySelectorAll<HTMLButtonElement>("li button")[index]!;
  await open(first.id);
  const viewer = setDocument.mock.instances.at(-1)! as InstanceType<typeof NoteViewer>;
  expect(inspectDocumentBacklinks).not.toHaveBeenCalled();
  panel.open = true;
  panel.dispatchEvent(new Event("toggle"));
  await vi.waitFor(() => expect(panel.querySelectorAll("li")).toHaveLength(3));
  expect(inspectDocumentBacklinks).toHaveBeenLastCalledWith("/resolved/root", "example", first.id, source, 30);
  expect(panel.textContent).toContain("Index or roadmap; unavailable");

  // Native summary and button key behavior must survive the Global scene's Tab shortcut.
  const summary = panel.querySelector("summary")!;
  summary.focus();
  const tab = new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true });
  summary.dispatchEvent(tab);
  expect(tab.defaultPrevented).toBe(false);
  expect(document.querySelector<HTMLElement>("#inventory-panel")!.hidden).toBe(true);
  for (const mode of ["live", "source", "reading"] as const) {
    viewer.setMode(mode);
    expect(button().disabled).toBe(false);
    expect(viewer.source).toBe(source);
  }

  button().click();
  expect(followDocumentBacklink).toHaveBeenLastCalledWith("/resolved/root", "example", first.id, source, second.id, 30);
  expect(viewer.source).toBe(source);
  pending[0]!.reject({ code: 4, message: "incoming edge removed" });
  await vi.waitFor(() => expect(status()).toContain("incoming edge removed"));
  expect(viewer.source).toBe(source);
  viewer.replaceSource(`draft\r\n${source}`);
  button().click();
  expect(followDocumentBacklink).toHaveBeenCalledOnce();
  expect(panel.textContent).toContain("Save or Discard");
  viewer.discard();

  button().click();
  viewer.replaceSource(`draft\r\n${source}`);
  pending[1]!.resolve(documents.get(second.id)!);
  await vi.waitFor(() => expect(status()).toContain("Unsaved changes"));
  expect(viewer.source).toBe(`draft\r\n${source}`);
  viewer.discard();
  button().click();
  await open(second.id);
  pending[2]!.resolve(documents.get(global.id)!);
  await Promise.resolve();
  expect(viewer.source).toBe("# Second\n");

  await open(first.id);
  await vi.waitFor(() => expect(button().disabled).toBe(false));
  button().click();
  pending[3]!.resolve(documents.get(global.id)!); // A mismatched wire cannot replace the viewer.
  await vi.waitFor(() => expect(status()).toContain("does not match"));
  expect(viewer.source).toBe(source);
  const normalLoads = loadDocumentWithRecovery.mock.calls.length;
  button().click();
  pending[4]!.resolve(documents.get(second.id)!);
  await vi.waitFor(() => expect(viewer.source).toBe("# Second\n"));
  expect(loadDocumentWithRecovery).toHaveBeenCalledTimes(normalLoads);

  await open(first.id);
  await vi.waitFor(() => expect(button().disabled).toBe(false));
  button(1).click();
  pending[5]!.resolve(documents.get(global.id)!);
  await vi.waitFor(() => expect(viewer.source).toBe("# Global\n"));
  expect(viewer.editable).toBe(false);
  document.querySelector<HTMLButtonElement>("#scope-toggle")!.click();
  await vi.waitFor(() => expect(document.querySelector(".pixel-stage")!.classList.contains("is-local")).toBe(true));
  await open(first.id);
  await vi.waitFor(() => expect(button().disabled).toBe(false));
  button(1).click();
  expect(status()).toContain("Switch to Global library");
  expect(followDocumentBacklink).toHaveBeenCalledTimes(6);
  button().click();
  document.querySelector<HTMLButtonElement>("#note-close")!.click();
  pending[6]!.reject(new Error("obsolete refusal after close"));
  await Promise.resolve();
  expect(document.querySelector<HTMLElement>("#note-overlay")!.hidden).toBe(true);
  expect(status()).not.toContain("obsolete refusal");
  viewer.destroy(); vi.restoreAllMocks(); vi.unstubAllGlobals();
});
