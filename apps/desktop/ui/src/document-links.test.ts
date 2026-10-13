// @vitest-environment jsdom
import { describe, expect, it, vi } from "vitest";
import { EditorView } from "@codemirror/view";
import { NoteViewer } from "./editor";
import { renderReading } from "./reading";
import type { LibraryDocumentLink } from "./types";

Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
Range.prototype.getBoundingClientRect = () => new DOMRect();

const raw = "![[Projects/example/entities/🌘.md#Here|Same alias]]";
const target = "Projects/example/entities/🌘.md";
function binding(source: string, kind: LibraryDocumentLink["kind"] = "canonical"): LibraryDocumentLink {
  const from = source.indexOf(raw);
  return { from, to: from + raw.length, label_from: source.indexOf("Same alias", from),
    label_to: from + raw.length - 2, target_id: target, kind };
}

describe("core-bound inline document controls", () => {
  for (const separator of ["\n", "\r\n"]) {
    it(`preserves exact ${JSON.stringify(separator)} source and activates Reading and Live Preview labels`, () => {
      const source = `---\nschema_version: 1\n---\n# 🌘\n**${raw}**\n`.replace(/\n/gu, separator);
      const host = document.createElement("div");
      document.body.append(host);
      const activate = vi.fn();
      const viewer = new NoteViewer(host, undefined, activate);
      const link = binding(source);
      viewer.setDocument(source, true);
      viewer.setLinks([link]);
      viewer.setMode("reading");
      const button = host.querySelector<HTMLButtonElement>(".reading-wikilink")!;
      expect(button.textContent).toBe("Same alias");
      expect(button.getAttribute("role")).toBe("link");
      expect(button.title).toContain(target);
      button.click();
      expect(activate).toHaveBeenLastCalledWith(link);
      viewer.setMode("live");
      const label = host.querySelector<HTMLElement>(".cm-document-link")!;
      expect(label.textContent).toBe("Same alias");
      expect(label.tabIndex).toBe(0);
      label.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      label.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
      label.dispatchEvent(new KeyboardEvent("keydown", { key: " ", bubbles: true, cancelable: true }));
      expect(activate).toHaveBeenCalledTimes(4);
      expect(viewer.source).toBe(source);
      expect(viewer.dirty).toBe(false);
      viewer.destroy(); host.remove();
    });
  }

  it("opens source occurrences only with a modifier and maps CRLF offsets", () => {
    const source = `🌘\r\n${raw}\r\n`;
    const host = document.createElement("div"); document.body.append(host);
    const activate = vi.fn();
    const viewer = new NoteViewer(host, undefined, activate);
    viewer.setDocument(source, true); viewer.setLinks([binding(source)]); viewer.setMode("source");
    const view = EditorView.findFromDOM(host.querySelector(".cm-editor")!)!;
    const cursor = source.indexOf("Same alias") - 1;
    view.dispatch({ selection: { anchor: cursor } });
    view.contentDOM.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", ctrlKey: true, bubbles: true, cancelable: true }));
    expect(activate).toHaveBeenLastCalledWith(binding(source));
    activate.mockClear();
    const coordinate = vi.spyOn(view, "posAtCoords").mockReturnValue(cursor);
    view.contentDOM.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    expect(activate).not.toHaveBeenCalled();
    view.contentDOM.dispatchEvent(new MouseEvent("click", { ctrlKey: true, bubbles: true, cancelable: true }));
    expect(activate).toHaveBeenLastCalledWith(binding(source));
    coordinate.mockRestore(); viewer.destroy(); host.remove();
  });

  it("removes stale bindings during edits and restores them on discard", () => {
    const source = `# 🌘\n${raw}\n`;
    const host = document.createElement("div");
    const activate = vi.fn();
    const viewer = new NoteViewer(host, undefined, activate);
    viewer.setDocument(source, true); viewer.setLinks([binding(source)]);
    viewer.replaceSource(`draft\n${source}`);
    expect(host.querySelector(".cm-document-link")).toBeNull();
    viewer.setMode("reading");
    expect(host.querySelector(".reading-wikilink")).toBeNull();
    expect(viewer.source).toBe(`draft\n${source}`);
    viewer.discard();
    host.querySelector<HTMLButtonElement>(".reading-wikilink")!.click();
    expect(activate).toHaveBeenCalledOnce();
    viewer.destroy();
  });

  it("never invents bindings for raw link-like text or executes HTML", () => {
    const host = document.createElement("div");
    renderReading(host, "[[unbound]] <img src=x onerror=alert(1)>\n`[[code]]`\n%% [[comment]] %%");
    expect(host.querySelector("[data-document-link]")).toBeNull();
    expect(host.querySelector("img")).toBeNull();
    expect(host.textContent).toContain("[[unbound]]");
  });

  it("presents non-book targets with a specific unavailable explanation", () => {
    for (const kind of ["projection", "other"] as const) {
      const host = document.createElement("div");
      const activate = vi.fn();
      const link = binding(raw, kind);
      renderReading(host, raw, [link], activate);
      const control = host.querySelector<HTMLButtonElement>(".reading-wikilink")!;
      expect(control.getAttribute("aria-disabled")).toBe("true");
      expect(control.title).toContain("not a library document");
      control.click();
      expect(activate).toHaveBeenCalledWith(link); // Shell explains refusal; no path read.
    }
  });
});
