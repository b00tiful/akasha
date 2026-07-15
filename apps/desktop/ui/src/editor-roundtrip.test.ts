// @vitest-environment jsdom

import { describe, expect, it } from "vitest";

import { NoteViewer } from "./editor";

Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
Range.prototype.getBoundingClientRect = () => new DOMRect();

const lfSource = `---
schema_version: 1
entity: editor-corpus
kind: fixture
status: active
reviewed: 2026-07-15
---

# Live Preview corpus

**Strong** and [[Global/entities/rust-pattern|a wikilink]].

![[Projects/example/entities/core#Section]]

\`inline **literal** markup\`

\`\`\`unknown-language
{{ unknown:: syntax }}
[[literal-code-link]]
\`\`\`

> [!note] Callout syntax remains canonical.

<!-- **comment markup** -->
%% [[obsidian-comment]] %%

  trailing spaces stay here.  
`;

const crlfSource = lfSource.replace(/\n/gu, "\r\n");

describe("editable CodeMirror round trips", () => {
  for (const [name, source] of [
    ["LF", lfSource],
    ["CRLF", crlfSource],
  ] as const) {
    it(`preserves ${name} canonical syntax, whitespace, and final newline`, () => {
      const host = document.createElement("div");
      const states: Array<{ dirty: boolean; editable: boolean }> = [];
      const viewer = new NoteViewer(host, (state) => states.push(state));
      viewer.setDocument(source, true);
      const separator = name === "CRLF" ? "\r\n" : "\n";
      const replacement = source.replace(
        `${separator}# Live Preview corpus`,
        `${separator}# Live Preview corpus${separator}${separator}Edited once.`,
      );

      viewer.replaceSource(replacement);
      viewer.setMode("source");
      viewer.setMode("reading");
      viewer.setMode("live");

      expect(viewer.source).toBe(replacement);
      expect(viewer.savedSource).toBe(source);
      expect(viewer.dirty).toBe(true);
      expect(states.at(-1)).toEqual({ dirty: true, editable: true });

      viewer.markSaved();
      expect(viewer.savedSource).toBe(replacement);
      expect(viewer.dirty).toBe(false);

      viewer.replaceSource(`${replacement}unsaved${separator}`);
      viewer.discard();
      expect(viewer.source).toBe(replacement);
      expect(viewer.dirty).toBe(false);
      viewer.destroy();
    });
  }

  it("keeps immutable selections read-only", () => {
    const host = document.createElement("div");
    const viewer = new NoteViewer(host);
    viewer.setDocument(lfSource, false);

    expect(() => viewer.replaceSource(`${lfSource}changed`)).toThrow("read-only");
    expect(viewer.source).toBe(lfSource);
    expect(viewer.dirty).toBe(false);
    viewer.destroy();
  });
});
