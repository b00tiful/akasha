// @vitest-environment jsdom
import { beforeEach, expect, it, vi } from "vitest";
import type { NoteLinksResult } from "./types";
import { BacklinkPanel, type BacklinkBinding } from "./backlinks";

const { inspect } = vi.hoisted(() => ({ inspect: vi.fn() }));
vi.mock("./api", () => ({ inspectDocumentBacklinks: inspect }));

const binding: BacklinkBinding = { root: "/resolved/root", project: "alpha", id: "Projects/alpha/entities/core.md", source: "🌘\r\n" };
function result(total = 1, count = total): NoteLinksResult {
  return { note: binding.id, direction: "incoming", scanned_canonical_notes: 180, scanned_projections: 8,
    total_matches: total, truncated: total > count,
    links: Array.from({ length: count }, (_, index) => ({ id: `Global/entities/source-${index}.md`, kind: "canonical" })),
  };
}
function panel() {
  const host = document.createElement("details");
  document.body.replaceChildren(host);
  const activate = vi.fn();
  const panel = new BacklinkPanel(host, activate);
  panel.bind(binding);
  const open = () => { host.open = true; host.dispatchEvent(new Event("toggle")); };
  const refresh = () => host.querySelector<HTMLButtonElement>("[data-refresh]")!.click();
  return { host, panel, activate, open, refresh };
}
beforeEach(() => { inspect.mockReset(); });

it("inspects on demand, reports exact counts and coverage, and explicitly expands to 100", async () => {
  inspect.mockImplementation(async (...args: unknown[]) => result(140, args[4] as number));
  const { host, open } = panel();
  expect(inspect).not.toHaveBeenCalled();
  open();
  await vi.waitFor(() => expect(host.textContent).toContain("Showing 30 of 140 incoming files; results truncated"));
  expect(inspect).toHaveBeenLastCalledWith(binding.root, binding.project, binding.id, binding.source, 30);
  expect(host.textContent).toContain("180 canonical notes and 8 indexes/roadmaps");
  expect(host.textContent).toContain("all registered projects and Global");
  expect(host.querySelectorAll("li")).toHaveLength(30);
  host.querySelector<HTMLButtonElement>("[data-expand]")!.click();
  await vi.waitFor(() => expect(host.querySelectorAll("li")).toHaveLength(100));
  expect(host.textContent).toContain("Showing 100 of 140");
  expect(host.querySelector<HTMLButtonElement>("[data-expand]")!.hidden).toBe(true);
});

it("shows empty results and unavailable projection identities without turning source text into markup", async () => {
  inspect.mockResolvedValueOnce(result(0));
  const { host, open, refresh, activate } = panel();
  open();
  await vi.waitFor(() => expect(host.textContent).toContain("No incoming links"));
  const next = result(2);
  next.links = [{ id: "Projects/alpha/index.md", kind: "projection" }, { id: "Global/entities/🌘<script>.md", kind: "canonical" }];
  inspect.mockResolvedValueOnce(next);
  refresh();
  await vi.waitFor(() => expect(host.querySelectorAll("li")).toHaveLength(2));
  expect(host.querySelectorAll("li button")).toHaveLength(1);
  expect(host.textContent).toContain("Index or roadmap; unavailable");
  expect(host.querySelector("script")).toBeNull();
  host.querySelector<HTMLButtonElement>("li button")!.click();
  expect(activate).toHaveBeenCalledWith(binding, next.links[1], 30);
});

it("retains stale results on failed refresh, disables opening, and allows a fresh retry", async () => {
  inspect.mockResolvedValueOnce(result());
  const { host, open, refresh, activate } = panel();
  open();
  await vi.waitFor(() => expect(host.querySelectorAll("li button")).toHaveLength(1));
  const old = host.querySelector<HTMLButtonElement>("li button")!;
  inspect.mockRejectedValueOnce({ code: 5, message: "pending journal" });
  refresh();
  expect(old.disabled).toBe(true);
  await vi.waitFor(() => expect(host.textContent).toContain("Previous results are stale"));
  expect(host.textContent).toContain("pending journal");
  old.click();
  expect(activate).not.toHaveBeenCalled();
  inspect.mockResolvedValueOnce(result());
  refresh();
  await vi.waitFor(() => expect(host.querySelector<HTMLButtonElement>("li button")!.disabled).toBe(false));
  old.click(); // Detached controls cannot authorize navigation after a fresh result.
  expect(activate).not.toHaveBeenCalled();
  host.querySelector<HTMLButtonElement>("li button")!.click();
  expect(activate).toHaveBeenCalledOnce();
});

it("lets dirty buffers inspect saved files while disabling navigation until discard", async () => {
  let resolve!: (result: NoteLinksResult) => void;
  inspect.mockImplementation(() => new Promise<NoteLinksResult>((done) => { resolve = done; }));
  const { host, panel: view, open, activate } = panel();
  open();
  view.setDirty(true);
  resolve(result());
  await vi.waitFor(() => expect(host.querySelectorAll("li button")).toHaveLength(1));
  expect(host.textContent).toContain("Unsaved changes: results describe saved files");
  const button = host.querySelector<HTMLButtonElement>("li button")!;
  button.click();
  expect(button.disabled).toBe(true);
  expect(activate).not.toHaveBeenCalled();
  view.setDirty(false);
  button.click();
  expect(activate).toHaveBeenCalledOnce();
});

it("ignores delayed success and failure after note, baseline, resolution or closed-view changes", async () => {
  const pending: Array<{ resolve(result: NoteLinksResult): void; reject(error: unknown): void }> = [];
  inspect.mockImplementation(() => new Promise<NoteLinksResult>((resolve, reject) => pending.push({ resolve, reject })));
  const { host, panel: view, open } = panel();
  open();
  const changed = { ...binding, root: "/another/root", source: "new baseline\n" };
  view.bind(changed); // Open panels inspect a freshly loaded binding, even for the same ID.
  pending[0]!.resolve(result());
  await Promise.resolve();
  expect(host.querySelectorAll("li")).toHaveLength(0);
  pending[1]!.resolve(result(0));
  await vi.waitFor(() => expect(host.textContent).toContain("No incoming links"));
  expect(view.isCurrent(binding)).toBe(false);
  view.bind({ ...changed, id: "Projects/alpha/entities/other.md" });
  view.bind(null);
  pending[2]!.reject(new Error("obsolete refusal"));
  await Promise.resolve();
  expect(host.textContent).toContain("Load a note");
  expect(host.textContent).not.toContain("obsolete refusal");
  expect(host.querySelectorAll("li")).toHaveLength(0);
});
