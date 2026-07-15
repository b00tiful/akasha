import { markdown } from "@codemirror/lang-markdown";
import { EditorState } from "@codemirror/state";
import {
  Decoration,
  type DecorationSet,
  EditorView,
  ViewPlugin,
  type ViewUpdate,
} from "@codemirror/view";
import { basicSetup } from "codemirror";

import { previewDecorations } from "./live-preview";
import { renderReading } from "./reading";

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

  constructor(host: HTMLElement, onStateChange: (state: NoteViewerState) => void = () => {}) {
    this.host = host;
    this.#onStateChange = onStateChange;
    this.render();
  }

  setDocument(source: string, editable = false): void {
    this.#source = source;
    this.#savedSource = source;
    this.#editable = editable;
    this.render();
    this.notifyState();
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
      renderReading(this.host, this.#source);
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
