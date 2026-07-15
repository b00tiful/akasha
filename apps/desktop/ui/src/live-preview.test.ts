// @vitest-environment jsdom

import { describe, expect, it } from "vitest";

import { previewDecorations } from "./live-preview";
import { renderReading } from "./reading";

const source = `---
schema_version: 1
entity: core
---

# **Core**

See [[Global/entities/pattern|shared pattern]].

\`\`\`md
[[literal-code-link]]
\`\`\`

> [!note] Unknown syntax remains visible.

\`**inline literal** [[inline-link]]\`
<!-- **HTML comment** -->
%% [[Obsidian/comment]] %%
\\**escaped strong** and \\[[escaped-link]]
`;

describe("first live-preview fidelity subset", () => {
  it("decorates headings, strong text, and wikilinks without touching canonical source", () => {
    const before = source;
    const decorations = previewDecorations(source, source.length);

    expect(decorations.some((item) => item.kind === "heading")).toBe(true);
    expect(decorations.some((item) => item.kind === "strong")).toBe(true);
    expect(decorations.some((item) => item.kind === "wikilink")).toBe(true);
    expect(
      decorations.some((item) => source.slice(item.from, item.to).includes("literal-code-link")),
    ).toBe(false);
    for (const literal of [
      "**inline literal** [[inline-link]]",
      "**HTML comment**",
      "[[Obsidian/comment]]",
      "**escaped strong**",
      "[[escaped-link]]",
    ]) {
      const from = source.indexOf(literal);
      const to = from + literal.length;
      expect(decorations.some((item) => item.from < to && item.to > from)).toBe(false);
    }
    expect(source).toBe(before);
  });

  it("reveals syntax on the active line", () => {
    const cursor = source.indexOf("Core");
    const lineStart = source.lastIndexOf("\n", cursor) + 1;
    const lineEnd = source.indexOf("\n", cursor);

    expect(
      previewDecorations(source, cursor).some(
        (item) => item.from >= lineStart && item.to <= lineEnd,
      ),
    ).toBe(false);
  });

  it("renders supported syntax safely and preserves unknown text", () => {
    const host = document.createElement("div");
    renderReading(host, source);

    expect(host.querySelector("h1")?.textContent).toBe("Core");
    expect(host.querySelector(".reading-wikilink")?.textContent).toBe("shared pattern");
    expect(host.textContent).toContain("[!note] Unknown syntax remains visible.");
    expect(host.textContent).toContain("[[literal-code-link]]");
  });
});
