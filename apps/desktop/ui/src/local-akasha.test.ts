// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ORBIT_PAGE_SIZE, orbitLayout, sectionLayout } from "./local-akasha-layout";
import { localAkashaModel } from "./local-akasha-model";
import { mountLocalAkasha, type LocalNavigation } from "./local-akasha-scene";
import { localSkyEvent, mountLocalSky } from "./local-akasha-sky";
import type { LibraryBook, LibraryProjection } from "./types";

export function localFixture(count = 53, sectionCount = 5): LibraryProjection {
  const books: LibraryBook[] = Array.from({ length: count }, (_, index) => ({
    id: `Projects/example/entities/note-${index}.md`, label: `Note ${index}`,
    scope: { kind: "project", project: "example" }, note_type: "section-0", class: "entity",
    status: "active", date: null, reviewed: null, outgoing_links: [], explanation: "Canonical fixture note",
  }));
  return {
    root: "/synthetic", selected_project: "example", global: { categories: [] },
    projects: [{ project: "example", status: "active", categories: Array.from({ length: sectionCount }, (_, index) => ({
      note_type: `section-${index}`, class: "entity", books: index === 0 ? books : [],
    })) }], total_books: count,
    dashboard: { validation_passed: true, projects: 1, notes: count, global_notes: 0,
      configured_categories: sectionCount, open_tasks: 0, open_problems: 0,
      validated_links: 0, latest_activity_date: null, project_metrics: [] },
  };
}

describe("Local Akasha hierarchy", () => {
  beforeEach(() => { vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null); });
  afterEach(() => { document.body.replaceChildren(); vi.restoreAllMocks(); vi.useRealTimers(); });

  function mount(count = 53, sectionCount = 5, reduced = true) {
    const model = localAkashaModel(localFixture(count, sectionCount));
    const host = document.createElement("div"); document.body.append(host);
    const navigation: LocalNavigation = { section: null, skyPage: 0, pages: {} };
    const callbacks = { canNavigate: vi.fn(() => true), onSelect: vi.fn(), onBack: vi.fn() };
    const handle = mountLocalAkasha(host, model, navigation, reduced, callbacks);
    return { host, model, navigation, callbacks, handle };
  }

  it("reuses exact core objects and excludes global and other projects", () => {
    const projection = localFixture();
    projection.projects.push({ ...projection.projects[0]!, project: "other" });
    projection.global.categories = projection.projects[0]!.categories;
    const model = localAkashaModel(projection);
    expect(model.noteCount).toBe(53);
    expect(model.sections).toBe(projection.projects[0]!.categories);
    projection.selected_project = "missing";
    expect(() => localAkashaModel(projection)).toThrow(/absent/);
  });

  it("has stable separated section positions regardless of input order", () => {
    const ids = Array.from({ length: 12 }, (_, i) => `section-${i}`);
    const first = sectionLayout("project", ids);
    expect(first).toEqual(sectionLayout("project", ids.reverse()));
    for (const [id, a] of first) for (const [other, b] of first) {
      if (id !== other) expect(Math.hypot(a.x - b.x, a.y - b.y)).toBeGreaterThan(.18);
    }
    expect(orbitLayout(Array.from({ length: 100 }, (_, i) => String(i))).size).toBe(ORBIT_PAGE_SIZE);
  });

  it("centers sparse and dense skies without relying on input order", () => {
    for (const count of [1, 3, 5, 8, 12]) for (const seed of ["alpha", "beta", "gamma"]) {
      const points = [...sectionLayout(seed, Array.from({ length: count }, (_, i) => `s${i}`)).values()];
      expect(points.reduce((sum, p) => sum + p.x, 0) / count).toBeCloseTo(.5);
      expect(points.reduce((sum, p) => sum + p.y, 0) / count).toBeCloseTo(.48);
      expect(points.every((p) => p.x > 0 && p.x < 1 && p.y > 0 && p.y < 1)).toBe(true);
    }
  });

  it("keeps events rare with quiet intervals and all three seeded event types", () => {
    const active = Array.from({ length: 200 }, (_, time) => localSkyEvent(time, "example"));
    expect(active.slice(0, 18).every((event) => event === null)).toBe(true);
    expect(active.filter(Boolean).length).toBeLessThan(36);
    expect(new Set(active.flatMap((event) => event ? [event.kind] : []))).toEqual(new Set(["comet", "rift", "eclipse"]));
  });

  it("stops the canvas clock for reader, reduced motion, hidden document, and teardown", () => {
    const context = { fillRect: vi.fn(), drawImage: vi.fn(), beginPath() {}, moveTo() {}, lineTo() {}, closePath() {}, fill() {} };
    vi.mocked(HTMLCanvasElement.prototype.getContext).mockReturnValue(context as unknown as CanvasRenderingContext2D);
    const hidden = vi.spyOn(document, "hidden", "get").mockReturnValue(false);
    const request = vi.spyOn(window, "requestAnimationFrame").mockReturnValue(123);
    const cancel = vi.spyOn(window, "cancelAnimationFrame");
    const sky = mountLocalSky(document.createElement("canvas"), "fixture", false);
    expect(request).toHaveBeenCalledTimes(1);
    sky.setPaused(true);
    expect(cancel).toHaveBeenLastCalledWith(123);
    expect(request).toHaveBeenCalledTimes(1);
    sky.setPaused(false);
    expect(request).toHaveBeenCalledTimes(2);
    sky.setReducedMotion(true);
    expect(request).toHaveBeenCalledTimes(2);
    hidden.mockReturnValue(true);
    sky.setReducedMotion(false);
    expect(request).toHaveBeenCalledTimes(2);
    sky.destroy();
    hidden.mockReturnValue(false);
    document.dispatchEvent(new Event("visibilitychange"));
    expect(request).toHaveBeenCalledTimes(2);
  });

  it("shows empty directories and empty configured sections truthfully", () => {
    const empty = mount(0, 0);
    expect(empty.host.textContent).toContain("No configured sections");
    empty.handle.destroy();
    const single = mount(0, 1);
    single.host.querySelector<HTMLButtonElement>(".local-section")!.click();
    expect(single.host.textContent).toContain("This section has no notes yet");
    expect(single.host.querySelectorAll(".local-note")).toHaveLength(0);
    single.handle.destroy();
  });

  it("visits all 53 notes in bounded orbital pages and restores the overview", () => {
    const { host, handle, navigation } = mount();
    const initial = [...host.querySelectorAll<HTMLElement>(".local-section")].map((node) => node.style.cssText);
    host.querySelector<HTMLButtonElement>('[data-section="section-0"]')!.click();
    const seen = new Set<string>();
    for (let page = 0; page < 4; page++) {
      const notes = [...host.querySelectorAll<HTMLElement>(".local-note")];
      expect(notes.length).toBeLessThanOrEqual(16);
      for (const note of notes) seen.add(note.dataset.note!);
      host.querySelector<HTMLButtonElement>(".local-cluster")?.click();
    }
    expect(seen.size).toBe(53);
    expect(navigation.pages["section-0"]).toBe(3);
    handle.back();
    expect([...host.querySelectorAll<HTMLElement>(".local-section")].map((node) => node.style.cssText)).toEqual(initial);
    expect((document.activeElement as HTMLElement).dataset.section).toBe("section-0");
    host.querySelector<HTMLButtonElement>('[data-section="section-0"]')!.click();
    expect(host.querySelectorAll(".local-note")).toHaveLength(5);
    handle.destroy();
  });

  it("keeps dozens of configured sections reachable without overlapping pages", () => {
    const { host, handle } = mount(1, 37);
    const ids = new Set<string>();
    for (let page = 0; page < 4; page++) {
      expect(host.querySelectorAll(".local-section").length).toBeLessThanOrEqual(12);
      host.querySelectorAll<HTMLElement>(".local-section").forEach((node) => ids.add(node.dataset.section!));
      host.querySelector<HTMLButtonElement>('[aria-label="Next orbital page"]')!.click();
    }
    expect(ids.size).toBe(37);
    handle.destroy();
  });

  it("refuses navigation when the editor is dirty and restores note focus on close", () => {
    const { host, callbacks, handle } = mount();
    host.querySelector<HTMLButtonElement>('[data-section="section-0"]')!.click();
    const notes = host.querySelectorAll<HTMLButtonElement>(".local-note");
    notes[3]!.click();
    const book = callbacks.onSelect.mock.calls[0]![0] as unknown as LibraryBook;
    handle.select(book.id);
    callbacks.canNavigate.mockReturnValue(false);
    handle.back();
    expect(host.querySelector(".local-akasha")!.getAttribute("data-phase")).toBe("note-open");
    handle.select(null);
    expect((document.activeElement as HTMLElement).dataset.note).toBe(book.id);
    handle.destroy();
  });

  it("locks transition input, resolves reduced motion immediately, and cancels teardown timers", () => {
    vi.useFakeTimers();
    const { host, handle } = mount(1, 5, false);
    host.querySelector<HTMLButtonElement>('[data-section="section-0"]')!.click();
    expect(host.querySelector(".local-akasha")!.getAttribute("data-phase")).toBe("entering-section");
    handle.setReducedMotion(true);
    expect(host.querySelector(".local-akasha")!.getAttribute("data-phase")).toBe("section");
    handle.setReducedMotion(false);
    handle.back();
    handle.destroy();
    vi.runAllTimers();
    expect(host.childElementCount).toBe(0);
  });
});
