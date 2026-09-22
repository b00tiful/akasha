// @vitest-environment jsdom
import { describe, expect, it, vi } from "vitest";
import shell from "../index.html?raw";
import type { DesktopLibrary, LibraryBook, LibraryDocument, LibrarySearchResult } from "./types";

describe("local scene and shared editor integration", () => {
  it("protects dirty edits, binds exact saves and searches, and ignores stale document responses", async () => {
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
        dashboard: { validation_passed: true, projects: 2, notes: 1001, global_notes: 0,
          configured_categories: 1, open_tasks: 99, open_problems: 98, validated_links: 97,
          latest_activity_date: null, project_metrics: [
            { project: "example", status: "active", notes: 2, populated_categories: 1,
              open_tasks: 3, open_problems: 4, validated_links: 5, latest_activity_date: null },
            { project: "other", status: "active", notes: 999, populated_categories: 1,
              open_tasks: 96, open_problems: 94, validated_links: 92, latest_activity_date: null },
          ] },
      },
    };
    const loads: Array<{ id: string; resolve(value: LibraryDocument): void }> = [];
    const loadDocument = vi.fn((_root: string, _project: string, id: string) =>
      new Promise<LibraryDocument>((resolve) => loads.push({ id, resolve })));
    const saveDocument = vi.fn(async () => ({ changed: true }));
    const searchProject = vi.fn(async (): Promise<LibrarySearchResult> => ({
      query: "second", scope: { kind: "project", project: "example" }, total_matches: 1,
      truncated: false, hits: [{ id: books[1]!.id, label: books[1]!.label,
        scope: books[1]!.scope, line: 2, snippet: "Second matching line" }],
    }));
    vi.doMock("./api", () => ({
      loadLibrary: vi.fn(async () => library), loadDocument, saveDocument, searchProject,
    }));
    vi.doMock("./scene", () => ({ mountLibraryScene: vi.fn(async () => ({
      aimedShelfId: () => null, select: vi.fn(), destroy: vi.fn(),
    })) }));
    const { NoteViewer } = await import("./editor");
    const documents = vi.spyOn(NoteViewer.prototype, "setDocument");
    await import("./main");
    await vi.waitFor(() => expect(document.querySelectorAll(".local-section")).toHaveLength(1));
    expect(document.querySelector(".brand-name")!.textContent).toBe("AKASHA LOCAL VAULT");
    expect(document.querySelector("#dashboard-title")!.textContent).toBe("Vault status");
    expect(document.querySelector("#dashboard-compact")!.textContent).toContain("Open tasks3");
    expect(document.querySelector("#dashboard-projects")!.textContent).not.toContain("other");
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
    click("#search-toggle");
    const searchInput = document.querySelector<HTMLInputElement>("#search-input")!;
    expect(document.activeElement).toBe(searchInput);
    searchInput.value = "second";
    document.querySelector<HTMLFormElement>("#search-form")!
      .dispatchEvent(new SubmitEvent("submit", { bubbles: true, cancelable: true }));
    await vi.waitFor(() => expect(searchProject).toHaveBeenCalledWith("/resolved/root", "example", "second"));
    await vi.waitFor(() => expect(document.querySelector("#search-summary")!.textContent).toContain("1 match"));
    expect(document.querySelector(".search-hit")!.textContent).toContain("Second matching line");
    click(".search-hit");
    await vi.waitFor(() => expect(loads).toHaveLength(4));
    loads[3]!.resolve({ id: books[1]!.id, source: "# Changed second\n" });
    await vi.waitFor(() => expect(document.querySelector<HTMLElement>("#search-panel")!.hidden).toBe(true));
    click("#note-close");
    click("#scope-toggle");
    await vi.waitFor(() => expect(document.querySelector("#dashboard-title")!.textContent).toBe("Library status"));
    expect(document.querySelector("#dashboard-compact")!.textContent).toContain("Open tasks99");
    expect(document.querySelector("#dashboard-projects")!.textContent).toContain("other");
    viewer.destroy();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });
});
