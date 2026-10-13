import { inspectDocumentBacklinks } from "./api";
import type { NoteLink, NoteLinksResult } from "./types";

export interface BacklinkBinding {
  root: string;
  project: string;
  id: string;
  source: string;
}

/** Presentation and request lifetime only. All graph policy lives in Rust. */
export class BacklinkPanel {
  private binding: BacklinkBinding | null = null;
  private request = 0;
  private limit = 30;
  private result: NoteLinksResult | null = null;
  private available = false;
  private busy = false;
  private dirty = false;
  private readonly summary: HTMLElement;
  private readonly message: HTMLElement;
  private readonly coverage: HTMLElement;
  private readonly guidance: HTMLElement;
  private readonly list: HTMLElement;
  private readonly refresh: HTMLButtonElement;
  private readonly expand: HTMLButtonElement;

  constructor(
    readonly host: HTMLDetailsElement,
    private readonly activate: (binding: BacklinkBinding, link: NoteLink, limit: number) => void,
  ) {
    host.innerHTML = `<summary>Backlinks</summary>
      <div class="backlink-controls"><button type="button" data-refresh>Refresh</button>
      <button type="button" data-expand hidden>Show up to 100</button></div>
      <p data-message role="status"></p><p data-coverage></p><p data-guidance></p>
      <ul class="backlink-list"></ul>`;
    this.summary = host.querySelector("summary")!;
    this.message = host.querySelector("[data-message]")!;
    this.coverage = host.querySelector("[data-coverage]")!;
    this.guidance = host.querySelector("[data-guidance]")!;
    this.list = host.querySelector("ul")!;
    this.refresh = host.querySelector("[data-refresh]")!;
    this.expand = host.querySelector("[data-expand]")!;
    host.addEventListener("toggle", () => {
      if (host.open && !this.result && !this.busy) void this.inspect(30);
    });
    this.refresh.addEventListener("click", () => void this.inspect(this.limit));
    this.expand.addEventListener("click", () => void this.inspect(100));
    this.bind(null);
  }

  bind(binding: BacklinkBinding | null): void {
    this.request++;
    this.binding = binding;
    this.limit = 30;
    this.result = null;
    this.available = false;
    this.busy = false;
    this.list.replaceChildren();
    this.coverage.textContent = "";
    this.summary.textContent = "Backlinks";
    this.message.textContent = binding ? "Open to inspect saved incoming links." : "Load a note to inspect backlinks.";
    this.updateControls();
    if (binding && this.host.open) void this.inspect(30);
  }

  isCurrent(binding: BacklinkBinding): boolean {
    return this.binding === binding;
  }

  setDirty(dirty: boolean): void {
    this.dirty = dirty;
    this.updateControls();
  }

  private updateControls(): void {
    this.refresh.disabled = !this.binding || this.busy;
    this.expand.disabled = this.refresh.disabled;
    this.expand.hidden = !this.result?.truncated || this.limit === 100;
    this.guidance.textContent = this.dirty
      ? "Unsaved changes: results describe saved files. Save or Discard before opening a backlink."
      : "Saved files across all registered projects and Global. Templates, Inbox, Meta and unregistered projects are excluded.";
    for (const button of this.list.querySelectorAll<HTMLButtonElement>("button")) {
      button.disabled = this.busy || !this.available || this.dirty;
    }
    this.host.setAttribute("aria-busy", String(this.busy));
  }

  private async inspect(limit: number): Promise<void> {
    const binding = this.binding;
    if (!binding) return;
    const request = ++this.request;
    this.busy = true;
    this.available = false;
    this.message.textContent = this.result
      ? "Refreshing saved backlinks; previous results retained below."
      : "Inspecting saved backlinks…";
    this.updateControls();
    try {
      const result = await inspectDocumentBacklinks(binding.root, binding.project, binding.id, binding.source, limit);
      if (request !== this.request || !this.isCurrent(binding)) return;
      if (result.note !== binding.id || result.direction !== "incoming") throw new Error("Backlink result identity does not match the displayed note.");
      this.result = result;
      this.limit = limit;
      this.available = true;
      this.summary.textContent = `Backlinks (${result.total_matches})`;
      this.message.textContent = result.total_matches === 0
        ? "No incoming links in the scanned saved files."
        : `Showing ${result.links.length} of ${result.total_matches} incoming files${result.truncated ? "; results truncated" : ""}.`;
      this.coverage.textContent = `Scanned ${result.scanned_canonical_notes} canonical notes and ${result.scanned_projections} indexes/roadmaps.`;
      this.list.replaceChildren(...result.links.map((link) => {
        const row = document.createElement("li");
        if (link.kind === "canonical") {
          const button = document.createElement("button");
          button.type = "button";
          button.textContent = link.id;
          button.addEventListener("click", () => {
            if (this.isCurrent(binding) && this.result === result && this.available && !this.busy && !this.dirty) {
              this.activate(binding, link, limit);
            }
          });
          row.append(button);
        } else {
          const identity = document.createElement("code");
          identity.textContent = link.id;
          const explanation = document.createElement("span");
          explanation.textContent = link.kind === "projection"
            ? " — Index or roadmap; unavailable as a library document."
            : " — Unavailable as a library document.";
          row.append(identity, explanation);
        }
        return row;
      }));
    } catch (error) {
      if (request !== this.request || !this.isCurrent(binding)) return;
      const message = typeof error === "object" && error !== null && "message" in error ? String(error.message) : String(error);
      this.message.textContent = `Backlinks unavailable: ${message}${this.result ? " Previous results are stale; refresh before opening." : ""}`;
    } finally {
      if (request === this.request && this.isCurrent(binding)) {
        this.busy = false;
        this.updateControls();
      }
    }
  }
}
