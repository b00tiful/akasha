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

export class NoteViewer {
  readonly host: HTMLElement;
  #source = "";
  #mode: ViewerMode = "live";
  #view: EditorView | null = null;

  constructor(host: HTMLElement) {
    this.host = host;
    this.render();
  }

  setDocument(source: string): void {
    this.#source = source;
    this.render();
  }

  setMode(mode: ViewerMode): void {
    this.#mode = mode;
    this.render();
  }

  get source(): string {
    return this.#source;
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
          EditorState.readOnly.of(true),
          EditorView.editable.of(false),
          EditorView.lineWrapping,
          this.#mode === "live" ? livePreviewPlugin : [],
          EditorView.theme({
            "&": { height: "100%", backgroundColor: "transparent" },
            ".cm-scroller": { overflow: "auto", fontFamily: "inherit" },
            ".cm-content": { padding: "16px" },
          }),
        ],
      }),
    });
  }
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
