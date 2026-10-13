import { markdown } from "@codemirror/lang-markdown";
import { EditorState, Prec } from "@codemirror/state";
import {
  Decoration,
  type DecorationSet,
  EditorView,
  ViewPlugin,
  type ViewUpdate,
} from "@codemirror/view";
import { basicSetup } from "codemirror";

import { previewDecorations } from "./live-preview";
import { linkTitle, renderReading } from "./reading";
import type { LibraryDocumentLink } from "./types";

export type ViewerMode = "live" | "source" | "reading";
export interface NoteViewerState {
  dirty: boolean;
  editable: boolean;
}

export class NoteViewer {
  readonly host: HTMLElement;
  #source = "";
  #savedSource = "";
  #editable = false;
  #mode: ViewerMode = "live";
  #view: EditorView | null = null;
  #onStateChange: (state: NoteViewerState) => void;
  #links: readonly LibraryDocumentLink[] = [];
  #onActivate: (link: LibraryDocumentLink) => void;

  constructor(
    host: HTMLElement, onStateChange: (state: NoteViewerState) => void = () => {},
    onActivate: (link: LibraryDocumentLink) => void = () => {},
  ) {
    this.host = host;
    this.#onStateChange = onStateChange;
    this.#onActivate = onActivate;
    this.render();
  }

  setDocument(source: string, editable = false): void {
    this.#source = source;
    this.#savedSource = source;
    this.#editable = editable;
    this.#links = [];
    this.render();
    this.notifyState();
  }

  setLinks(links: readonly LibraryDocumentLink[]): void {
    this.#links = links;
    this.render();
  }

  setMode(mode: ViewerMode): void {
    this.#mode = mode;
    this.render();
  }

  get source(): string {
    return this.#source;
  }

  get savedSource(): string {
    return this.#savedSource;
  }

  get dirty(): boolean {
    return this.#source !== this.#savedSource;
  }

  get editable(): boolean {
    return this.#editable;
  }

  replaceSource(source: string): void {
    if (!this.#editable) {
      throw new Error("the selected note is read-only");
    }
    if (!this.#view) {
      this.#source = source;
      this.render();
      this.notifyState();
      return;
    }
    this.#view.dispatch({
      changes: { from: 0, to: this.#view.state.doc.length, insert: source },
    });
  }

  markSaved(): void {
    this.#savedSource = this.#source;
    this.notifyState();
  }

  discard(): void {
    if (!this.dirty) {
      return;
    }
    this.#source = this.#savedSource;
    this.render();
    this.notifyState();
  }

  destroy(): void {
    this.#view?.destroy();
    this.#view = null;
    this.host.replaceChildren();
  }

  private render(): void {
    this.#view?.destroy();
    this.#view = null;
    this.host.replaceChildren();
    if (this.#mode === "reading") {
      renderReading(this.host, this.#source, this.dirty ? [] : this.#links, this.#onActivate);
      return;
    }

    this.#view = new EditorView({
      parent: this.host,
      state: EditorState.create({
        doc: this.#source,
        extensions: [
          basicSetup,
          markdown(),
          EditorState.lineSeparator.of(lineSeparator(this.#source)),
          EditorState.readOnly.of(!this.#editable),
          EditorView.editable.of(this.#editable),
          EditorView.lineWrapping,
          this.#mode === "live" ? livePreviewPlugin : [],
          Prec.highest(this.linkPlugin()),
          EditorView.updateListener.of((update) => {
            if (update.docChanged) {
              this.#source = update.state.sliceDoc();
              this.notifyState();
            }
          }),
          EditorView.theme({
            "&": { height: "100%", backgroundColor: "transparent" },
            ".cm-scroller": { overflow: "auto", fontFamily: "inherit" },
            ".cm-content": { padding: "16px" },
          }),
        ],
      }),
    });
  }

  private linkPlugin() {
    const viewer = this;
    // CodeMirror normalizes line endings internally; wire offsets refer to exact source.
    const position = (offset: number) => viewer.#savedSource.slice(0, offset).replace(/\r\n/gu, "\n").length;
    const activate = (link: LibraryDocumentLink) => viewer.#onActivate(link);
    return ViewPlugin.fromClass(class {
      decorations: DecorationSet = Decoration.none;
      constructor(view: EditorView) { this.build(view); }
      update(update: ViewUpdate) { if (update.docChanged || update.selectionSet) this.build(update.view); }
      build(view: EditorView) {
        if (view.state.sliceDoc() !== viewer.#savedSource || viewer.#mode !== "live") {
          this.decorations = Decoration.none;
          return;
        }
        this.decorations = Decoration.set(viewer.#links.map((link) => Decoration.mark({
          class: "cm-document-link",
          attributes: { "data-document-link": String(link.from), role: "link", tabindex: "0",
            title: linkTitle(link), "aria-label": linkTitle(link),
            "aria-disabled": String(link.kind !== "canonical") },
        }).range(position(link.label_from), position(link.label_to))), true);
      }
    }, {
      decorations: (plugin) => plugin.decorations,
      eventHandlers: {
        click(event, view) {
          if (viewer.dirty) return false;
          const element = event.target instanceof Element ? event.target.closest<HTMLElement>("[data-document-link]") : null;
          const offset = view.posAtCoords({ x: event.clientX, y: event.clientY });
          const link = viewer.#links.find((link) => element
            ? link.from === Number(element.dataset.documentLink)
            : (event.ctrlKey || event.metaKey) && offset !== null && offset >= position(link.from) && offset < position(link.to));
          if (!link || (viewer.#mode === "source" && !event.ctrlKey && !event.metaKey)) return false;
          event.preventDefault();
          activate(link);
          return true;
        },
        keydown(event, view) {
          if (viewer.dirty) return false;
          const element = event.target instanceof Element ? event.target.closest<HTMLElement>("[data-document-link]") : null;
          if (!(element && (event.key === "Enter" || event.key === " ")) &&
              !((event.ctrlKey || event.metaKey) && event.key === "Enter")) return false;
          const link = viewer.#links.find((link) => element
            ? link.from === Number(element.dataset.documentLink)
            : view.state.selection.main.head >= position(link.from) && view.state.selection.main.head < position(link.to));
          if (!link) return false;
          event.preventDefault();
          activate(link);
          return true;
        },
      },
    });
  }

  private notifyState(): void {
    this.#onStateChange({ dirty: this.dirty, editable: this.#editable });
  }
}

function lineSeparator(source: string): "\n" | "\r\n" {
  const withoutCrLf = source.replace(/\r\n/gu, "");
  return source.includes("\r\n") && !withoutCrLf.includes("\n") ? "\r\n" : "\n";
}

const livePreviewPlugin = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;

    constructor(view: EditorView) {
      this.decorations = buildDecorations(view);
    }

    update(update: ViewUpdate): void {
      if (update.docChanged || update.selectionSet || update.viewportChanged) {
        this.decorations = buildDecorations(update.view);
      }
    }
  },
  { decorations: (plugin) => plugin.decorations },
);

function buildDecorations(view: EditorView): DecorationSet {
  const source = view.state.doc.toString();
  const cursor = view.state.selection.main.head;
  const ranges = previewDecorations(source, cursor).map((item) => {
    if (item.kind === "syntax") {
      return Decoration.replace({}).range(item.from, item.to);
    }
    return Decoration.mark({ class: `cm-live-${item.kind}` }).range(item.from, item.to);
  });
  return Decoration.set(ranges, true);
}
