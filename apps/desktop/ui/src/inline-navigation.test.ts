// @vitest-environment jsdom
import { expect, it, vi } from "vitest";
import shell from "../index.html?raw";
import type { DesktopLibrary, LibraryBook, LibraryDocumentLink, LinkedLibraryDocument } from "./types";

it("guards inline activation against dirty, stale, unknown, cross-scope and obsolete responses", async () => {
  document.body.innerHTML = new DOMParser().parseFromString(shell, "text/html").body.innerHTML;
  history.replaceState(null, "", "/");
  vi.stubGlobal("matchMedia", () => ({ matches: true, addListener: vi.fn(), removeListener: vi.fn(),
    addEventListener: vi.fn(), removeEventListener: vi.fn() }));
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
  Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
  Range.prototype.getBoundingClientRect = () => new DOMRect();
  const books: LibraryBook[] = ["first", "second", "global"].map((name) => ({
    id: name === "global" ? "Global/entities/shared.md" : `Projects/example/entities/${name}.md`,
    label: name, scope: name === "global" ? { kind: "global" } : { kind: "project", project: "example" },
    note_type: "entity", class: "entity", status: "active", reviewed: null, date: null,
    outgoing_links: [], explanation: name,
  }));
  const first = books[0]!, second = books[1]!, global = books[2]!;
  const rawLinks = [
    { target_id: second.id, kind: "canonical" },
    { target_id: global.id, kind: "canonical" },
    { target_id: "Projects/example/index.md", kind: "projection" },
    { target_id: "Projects/example/entities/absent.md", kind: "canonical" },
  ] as const;
  const source = `# 🌘 First\r\n${rawLinks.map((link) => `[[${link.target_id}|same]]`).join("\r\n")}\r\n`;
  const links: LibraryDocumentLink[] = rawLinks.map((link) => {
    const from = source.indexOf(`[[${link.target_id}`);
    return { ...link, from, to: source.indexOf("]]", from) + 2,
      label_from: source.indexOf("|same", from) + 1, label_to: source.indexOf("]]", from) };
  });
  const documents = new Map<string, LinkedLibraryDocument>([
    [first.id, { id: first.id, source, links }],
    [second.id, { id: second.id, source: "# Second\n", links: [] }],
    [global.id, { id: global.id, source: "# Shared\n", links: [] }],
  ]);
  const data: DesktopLibrary = { recovery: "none", fallback_markdown: "", projection: {
    root: "/resolved/root", selected_project: "example", total_books: 3,
    global: { categories: [{ note_type: "entity", class: "entity", books: [global] }] },
    projects: [{ project: "example", status: "active", categories: [{ note_type: "entity", class: "entity", books: [first, second] }] }],
    dashboard: { validation_passed: true, projects: 1, notes: 3, global_notes: 1, configured_categories: 1,
      open_tasks: 0, open_problems: 0, validated_links: 4, latest_activity_date: null,
      project_metrics: [{ project: "example", status: "active", notes: 2, populated_categories: 1,
        open_tasks: 0, open_problems: 0, validated_links: 4, latest_activity_date: null }] },
  } };
  const pending: Array<{ resolve(value: LinkedLibraryDocument): void; reject(error: unknown): void }> = [];
  const followDocumentLink = vi.fn(() => new Promise<LinkedLibraryDocument>((resolve, reject) => pending.push({ resolve, reject })));
  const loadDocumentWithRecovery = vi.fn(async (_root: string, _project: string, id: string) => documents.get(id)!);
  vi.doMock("./api", () => ({ followDocumentLink, loadDocumentWithRecovery,
    loadLibraryWithRecovery: vi.fn(async () => data), loadLocalNavigation: vi.fn(async () => null),
    saveLocalNavigation: vi.fn(async () => undefined), inspectRecovery: vi.fn(),
    saveDocument: vi.fn(), searchProject: vi.fn(),
  }));
  vi.doMock("./scene", () => ({ mountLibraryScene: vi.fn(async () => ({
    aimedShelfId: () => null, select: vi.fn(), destroy: vi.fn(), aimShelf: vi.fn(),
    activateShelf: vi.fn(), openVolume: vi.fn(),
  })) }));
  const { NoteViewer } = await import("./editor");
  const setDocument = vi.spyOn(NoteViewer.prototype, "setDocument");
  await import("./main");
  await vi.waitFor(() => expect(document.querySelectorAll(".book-button")).toHaveLength(3));
  const status = () => document.querySelector("#status")!.textContent;
  const open = async (id: string) => {
    [...document.querySelectorAll<HTMLButtonElement>(".book-button")].find((button) => button.textContent === id)!.click();
    await vi.waitFor(() => expect(setDocument).toHaveBeenLastCalledWith(documents.get(id)!.source, id !== global.id));
  };
  await open(first.id);
  const viewer = setDocument.mock.instances.at(-1)! as InstanceType<typeof NoteViewer>;
  viewer.setMode("reading");
  const control = (index = 0) => document.querySelectorAll<HTMLButtonElement>(".reading-wikilink")[index]!;
  control().click();
  expect(followDocumentLink).toHaveBeenLastCalledWith("/resolved/root", "example", first.id, source, links[0]!.from);
  expect(viewer.source).toBe(source); // Keep source visible while pending.
  pending[0]!.reject({ code: 5, message: "saved source changed; reload" });
  await vi.waitFor(() => expect(status()).toContain("Link unavailable"));
  expect(viewer.source).toBe(source);

  const oldControl = control();
  viewer.replaceSource(`draft\r\n${source}`);
  oldControl.click();
  expect(followDocumentLink).toHaveBeenCalledOnce();
  expect(status()).toContain("Unsaved changes");
  expect(viewer.source).toBe(`draft\r\n${source}`);
  viewer.discard();
  control().click();
  viewer.replaceSource(`draft\r\n${source}`);
  pending[1]!.resolve(documents.get(second.id)!);
  await vi.waitFor(() => expect(status()).toContain("Unsaved changes"));
  expect(viewer.dirty).toBe(true);
  expect(viewer.source).toBe(`draft\r\n${source}`);
  viewer.discard();

  control().click();
  await open(second.id);
  pending[2]!.resolve(documents.get(global.id)!);
  await Promise.resolve();
  expect(viewer.source).toBe("# Second\n");
  await open(first.id);
  control(2).click();
  expect(status()).toContain("Index or roadmap is not a library document");
  control(3).click();
  expect(status()).toContain("absent from this library");
  expect(followDocumentLink).toHaveBeenCalledTimes(3);
  control().click();
  pending[3]!.resolve(documents.get(global.id)!); // Mismatched response must retain source.
  await vi.waitFor(() => expect(status()).toContain("does not match"));
  expect(viewer.source).toBe(source);

  control().click();
  pending[4]!.resolve(documents.get(second.id)!);
  await vi.waitFor(() => expect(viewer.source).toBe("# Second\n"));
  expect(loadDocumentWithRecovery).toHaveBeenCalledTimes(3); // Activation uses the checked result directly.
  await open(first.id);
  control(1).click();
  pending[5]!.resolve(documents.get(global.id)!);
  await vi.waitFor(() => expect(viewer.source).toBe("# Shared\n"));
  expect(viewer.editable).toBe(false);

  document.querySelector<HTMLButtonElement>("#scope-toggle")!.click();
  await vi.waitFor(() => expect(document.querySelector(".pixel-stage")!.classList.contains("is-local")).toBe(true));
  await open(first.id);
  control(1).click();
  expect(status()).toContain("Switch to Global library");
  expect(followDocumentLink).toHaveBeenCalledTimes(6);
  expect(viewer.source).toBe(source);
  viewer.destroy(); vi.restoreAllMocks(); vi.unstubAllGlobals();
});
