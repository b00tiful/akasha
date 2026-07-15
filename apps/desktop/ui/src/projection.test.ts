import { describe, expect, it } from "vitest";

import { allBooks, layoutBooks, visibleLinks } from "./projection";
import type { LibraryProjection } from "./types";

const projection: LibraryProjection = {
  root: "/synthetic",
  selected_project: "alpha",
  global: {
    categories: [
      {
        note_type: "entity",
        class: "entity",
        books: [book("Global/entities/pattern.md", ["Projects/alpha/entities/core.md"])],
      },
    ],
  },
  projects: [
    {
      project: "alpha",
      status: "active",
      categories: [
        {
          note_type: "entity",
          class: "entity",
          books: [
            book("Projects/alpha/entities/core.md", [
              "Projects/alpha/index.md",
              "Global/entities/pattern.md",
            ]),
          ],
        },
      ],
    },
  ],
  total_books: 2,
};

describe("desktop projection helpers", () => {
  it("keeps canonical identities supplied by the Rust projection", () => {
    expect(allBooks(projection).map((item) => item.id)).toEqual([
      "Global/entities/pattern.md",
      "Projects/alpha/entities/core.md",
    ]);
    expect(layoutBooks(projection).map((item) => item.book.id)).toEqual(
      allBooks(projection).map((item) => item.id),
    );
  });

  it("renders traffic only when both validated books are visible", () => {
    expect(visibleLinks(projection)).toEqual([
      {
        source: "Global/entities/pattern.md",
        target: "Projects/alpha/entities/core.md",
      },
      {
        source: "Projects/alpha/entities/core.md",
        target: "Global/entities/pattern.md",
      },
    ]);
  });

  it("keeps five configured categories inside the visible shelf", () => {
    const categories = Array.from({ length: 5 }, (_, index) => ({
      note_type: `type-${index}`,
      class: "entity" as const,
      books: [book(`Projects/alpha/entities/book-${index}.md`, [])],
    }));
    const crowded: LibraryProjection = {
      ...projection,
      global: { categories: [] },
      projects: [{ project: "alpha", status: "active", categories }],
      total_books: categories.length,
    };

    const positions = layoutBooks(crowded);

    expect(positions.map((item) => item.y)).toEqual([74, 127, 180, 233, 286]);
    expect(Math.max(...positions.map((item) => item.y)) + 24).toBeLessThanOrEqual(314);
  });
});

function book(id: string, outgoing_links: string[]) {
  return {
    id,
    label: id.split("/").at(-1)?.replace(/\.md$/u, "") ?? id,
    scope: id.startsWith("Global/")
      ? ({ kind: "global" } as const)
      : ({ kind: "project", project: "alpha" } as const),
    note_type: "entity",
    class: "entity" as const,
    status: "active",
    reviewed: "2026-07-13",
    date: null,
    outgoing_links,
    explanation: id,
  };
}
