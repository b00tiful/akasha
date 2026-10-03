// @vitest-environment jsdom
import { expect, it, vi } from "vitest";
import shell from "../index.html?raw";
import type { DesktopLibrary } from "./types";

it("replaces a previously loaded library with exact-text recovery guidance on refusal and retry", async () => {
  document.body.innerHTML = new DOMParser().parseFromString(shell, "text/html").body.innerHTML;
  history.replaceState(null, "", "/");
  vi.stubGlobal("matchMedia", () => ({ matches: true, addEventListener: vi.fn(),
    removeEventListener: vi.fn(), addListener: vi.fn(), removeListener: vi.fn() }));
  Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
  Range.prototype.getBoundingClientRect = () => new DOMRect();
  const library: DesktopLibrary = {
    recovery: "none", fallback_markdown: "", projection: {
      root: "/resolved/root", selected_project: "example", total_books: 0,
      global: { categories: [] }, projects: [{ project: "example", status: "active",
        categories: [{ note_type: "entity", class: "entity", books: [] }] }],
      dashboard: { validation_passed: true, projects: 1, notes: 0, global_notes: 0,
        configured_categories: 1, open_tasks: 0, open_problems: 0, validated_links: 0,
        latest_activity_date: null, project_metrics: [{ project: "example", status: "active",
          notes: 0, populated_categories: 0, open_tasks: 0, open_problems: 0,
          validated_links: 0, latest_activity_date: null }] },
    },
  };
  const failure = { code: 5, message: "unexpected <external> bytes; automatic recovery refused" };
  const load = vi.fn().mockResolvedValueOnce(library).mockRejectedValue(failure);
  const journal = "/requested/<project>/.akasha-edit-journal.json";
  const inspect = vi.fn(async () => ({ project: "example", journal_path: journal, pending: true }));
  vi.doMock("./api", () => ({
    loadLibraryWithRecovery: load, inspectRecovery: inspect,
    loadDocumentWithRecovery: vi.fn(), saveDocument: vi.fn(), searchProject: vi.fn(),
    loadLocalNavigation: vi.fn(async () => null), saveLocalNavigation: vi.fn(),
  }));
  const destroy = vi.fn();
  vi.doMock("./scene", () => ({ mountLibraryScene: vi.fn(async () => ({
    aimedShelfId: () => null, select: vi.fn(), destroy,
  })) }));
  const { NoteViewer } = await import("./editor");
  const documents = vi.spyOn(NoteViewer.prototype, "setDocument");
  await import("./main");
  await vi.waitFor(() => expect(document.querySelector("#status")!.textContent).toContain("validation passed"));
  expect(document.querySelector<HTMLElement>("#dashboard")!.hidden).toBe(false);
  document.querySelector<HTMLInputElement>("#root-input")!.value = "/requested/root";
  const form = document.querySelector<HTMLFormElement>("#library-form")!;
  for (const count of [1, 2]) {
    form.requestSubmit();
    await vi.waitFor(() => expect(inspect).toHaveBeenCalledTimes(count));
    await vi.waitFor(() => expect(form.classList.contains("is-loading")).toBe(false));
    expect(inspect).toHaveBeenLastCalledWith("/requested/root", "example");
    expect(document.querySelector("#status")!.textContent).toContain(failure.message);
    expect(document.querySelector("#inventory-title")!.textContent).toBe("Recovery inspection");
    expect(document.querySelector<HTMLElement>("#inventory-panel")!.hidden).toBe(false);
    expect(document.querySelector("#fallback")!.textContent).toContain(journal);
    expect(document.querySelector("#fallback")!.textContent).toContain("Keep the journal");
    expect(document.querySelector("#fallback external, #fallback project")).toBeNull();
    expect(document.querySelector<HTMLElement>("#dashboard")!.hidden).toBe(true);
    expect(document.querySelector<HTMLElement>("#note-overlay")!.hidden).toBe(true);
    expect(document.querySelector("#scene")!.childElementCount).toBe(0);
    expect(documents).toHaveBeenLastCalledWith("", false);
  }
  expect(destroy).toHaveBeenCalledOnce();
  (documents.mock.instances.at(-1)! as InstanceType<typeof NoteViewer>).destroy();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});
