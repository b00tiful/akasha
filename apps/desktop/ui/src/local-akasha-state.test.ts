import { describe, expect, it } from "vitest";
import { ORBIT_PAGE_SIZE } from "./local-akasha-layout";
import { localAkashaModel } from "./local-akasha-model";
import {
  emptyLocalNavigation,
  restoreLocalNavigation,
  snapshotLocalNavigation,
} from "./local-akasha-state";
import type { LibraryBook, LibraryProjection, LocalNavigationState } from "./types";

function localFixture(count: number, sectionCount: number): LibraryProjection {
  const books: LibraryBook[] = Array.from({ length: count }, (_, index) => ({
    id: `Projects/example/entities/note-${index}.md`,
    label: `Note ${index}`,
    scope: { kind: "project", project: "example" },
    note_type: "section-0",
    class: "entity",
    status: "active",
    reviewed: null,
    date: null,
    outgoing_links: [],
    explanation: "fixture",
  }));
  return {
    root: "/synthetic",
    selected_project: "example",
    global: { categories: [] },
    projects: [{
      project: "example",
      status: "active",
      categories: Array.from({ length: sectionCount }, (_, index) => ({
        note_type: `section-${index}`,
        class: "entity" as const,
        books: index === 0 ? books : [],
      })),
    }],
    total_books: count,
    dashboard: {
      validation_passed: true,
      projects: 1,
      notes: count,
      global_notes: 0,
      configured_categories: sectionCount,
      open_tasks: 0,
      open_problems: 0,
      validated_links: 0,
      latest_activity_date: null,
      project_metrics: [],
    },
  };
}

describe("Local Akasha persistent navigation", () => {
  it("stores stable section and note anchors instead of page indexes or transient content", () => {
    const model = localAkashaModel(localFixture(53, 20));
    const state = snapshotLocalNavigation(model, {
      section: "section-0",
      skyPage: 1,
      pages: { "section-0": 2, missing: 99 },
    }, "Projects/example/entities/note-35.md");

    expect(state).toEqual({
      version: 1,
      root: "/synthetic",
      project: "example",
      section: "section-0",
      sky_anchor: "section-2",
      page_anchors: { "section-0": `Projects/example/entities/note-${ORBIT_PAGE_SIZE * 2}.md` },
      note: "Projects/example/entities/note-35.md",
    });
    expect(JSON.stringify(state)).not.toMatch(/source|draft|query|skyPage|pageIndex/u);
  });

  it("revalidates anchors against fresh projection data and derives current pages", () => {
    const model = localAkashaModel(localFixture(53, 20));
    const state: LocalNavigationState = {
      version: 1,
      root: model.root,
      project: model.project,
      section: "section-4",
      sky_anchor: "section-12",
      page_anchors: {
        "section-0": "Projects/example/entities/note-32.md",
        "section-4": "Projects/example/entities/missing.md",
        missing: "Projects/example/entities/note-0.md",
      },
      note: "Projects/example/entities/note-35.md",
    };

    const restored = restoreLocalNavigation(model, state);
    expect(restored.note?.id).toBe(state.note);
    expect(restored.navigation.section).toBe("section-0");
    expect(restored.navigation.skyPage).toBe(0);
    expect(restored.navigation.pages).toEqual({ "section-0": 2 });
  });

  it("falls back to the nearest valid parent when identities are stale or misbound", () => {
    const model = localAkashaModel(localFixture(10, 20));
    const stale: LocalNavigationState = {
      version: 1,
      root: model.root,
      project: model.project,
      section: "removed",
      sky_anchor: "section-12",
      page_anchors: { removed: "Projects/example/entities/note-0.md" },
      note: "Projects/example/entities/removed.md",
    };
    expect(restoreLocalNavigation(model, stale)).toEqual({
      navigation: { section: null, skyPage: 0, pages: {} },
      note: null,
    });
    expect(restoreLocalNavigation(model, { ...stale, root: "/other" })).toEqual({
      navigation: emptyLocalNavigation(),
      note: null,
    });
  });
});
