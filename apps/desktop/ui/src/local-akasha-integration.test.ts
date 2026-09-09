// @vitest-environment jsdom
import { describe, expect, it, vi } from "vitest";
import shell from "../index.html?raw";
import type { DesktopLibrary, LibraryBook, LibraryDocument } from "./types";

describe("local scene and shared editor integration", () => {
  it("protects dirty edits, binds saves to resolved identity, and ignores stale document responses", async () => {
    document.body.innerHTML = new DOMParser().parseFromString(shell, "text/html").body.innerHTML;
    history.replaceState(null, "", "/?view=local");
    vi.stubGlobal("matchMedia", () => ({ matches: true, addEventListener: vi.fn(),
      removeEventListener: vi.fn(), addListener: vi.fn(), removeListener: vi.fn() }));
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
    Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
    Range.prototype.getBoundingClientRect = () => new DOMRect();
    const books: LibraryBook[] = ["first", "second"].map((name) => ({
      id: `Projects/example/entities/${name}.md`, label: name,
      scope: { kind: "project", project: "example" }, note_type: "entity", class: "entity",
      status: "active", reviewed: null, date: null, outgoing_links: [], explanation: name,
    }));
    const library: DesktopLibrary = {
      recovery: "none", fallback_markdown: "", projection: {
        root: "/resolved/root", selected_project: "example", total_books: 2,
        global: { categories: [] }, projects: [{ project: "example", status: "active",
          categories: [{ note_type: "entity", class: "entity", books }] }],
        dashboard: { validation_passed: true, projects: 1, notes: 2, global_notes: 0,
          configured_categories: 1, open_tasks: 0, open_problems: 0, validated_links: 0,
          latest_activity_date: null, project_metrics: [] },
      },
    };
    const loads: Array<{ id: string; resolve(value: LibraryDocument): void }> = [];
    const loadDocument = vi.fn((_root: string, _project: string, id: string) =>
      new Promise<LibraryDocument>((resolve) => loads.push({ id, resolve })));
    const saveDocument = vi.fn(async () => ({ changed: true }));
    vi.doMock("./api", () => ({ loadLibrary: vi.fn(async () => library), loadDocument, saveDocument }));
    vi.doMock("./scene", () => ({ mountLibraryScene: vi.fn() }));
    const { NoteViewer } = await import("./editor");
    const documents = vi.spyOn(NoteViewer.prototype, "setDocument");
    await import("./main");
    await vi.waitFor(() => expect(document.querySelectorAll(".local-section")).toHaveLength(1));
    const click = (selector: string) => document.querySelector<HTMLButtonElement>(selector)!.click();
    click(".local-section");
    click(".local-note");
    click(".local-note:nth-of-type(2)");
    expect(loads).toHaveLength(2);
    loads[1]!.resolve({ id: books[1]!.id, source: "# Second\n" });
    await vi.waitFor(() => expect(documents).toHaveBeenLastCalledWith("# Second\n", true));
    loads[0]!.resolve({ id: books[0]!.id, source: "# Stale first\n" });
    await Promise.resolve();
    expect(documents).toHaveBeenLastCalledWith("# Second\n", true);
    const viewer = documents.mock.instances.at(-1)! as InstanceType<typeof NoteViewer>;
    viewer.replaceSource("# Changed second\n");
    click("#scope-toggle");
    expect(document.querySelector(".pixel-stage")!.classList.contains("is-local")).toBe(true);
    expect(document.querySelector("#status")!.textContent).toContain("Unsaved changes");
    click(".local-note");
    expect(loads).toHaveLength(2);
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    expect(document.querySelector<HTMLElement>("#note-overlay")!.hidden).toBe(false);
    const tab = new KeyboardEvent("keydown", { key: "Tab", cancelable: true });
    window.dispatchEvent(tab);
    expect(tab.defaultPrevented).toBe(false);
    (document.querySelector("#root-input") as HTMLInputElement).value = "/unsubmitted/path";
    click("#save-note");
    await vi.waitFor(() => expect(saveDocument).toHaveBeenCalledWith(
      "/resolved/root", "example", books[1]!.id, "# Second\n", "# Changed second\n",
    ));
    await vi.waitFor(() => expect(loads).toHaveLength(3));
    loads[2]!.resolve({ id: books[1]!.id, source: "# Changed second\n" });
    await vi.waitFor(() => expect(documents).toHaveBeenLastCalledWith("# Changed second\n", true));
    click("#note-close");
    expect((document.activeElement as HTMLElement).dataset.note).toBe(books[1]!.id);
    expect(document.querySelector(".local-akasha")!.getAttribute("data-phase")).toBe("section");
    viewer.destroy();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });
});
