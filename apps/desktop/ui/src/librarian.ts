import { inspectLibrarian, followLibrarianSource } from "./api";
import type { DesktopLibrarianReview, LibrarianNoteProvenance } from "./librarian-types";
import type { LinkedLibraryDocument } from "./types";

interface Binding { root: string; project: string }

/** Ephemeral presentation and obsolete-request suppression; Rust owns every source check. */
export class LibrarianPanel {
  private binding: Binding | null = null;
  private generation = 0;
  private result: DesktopLibrarianReview | null = null;
  private available = false;
  private busy = false;
  private opening = false;
  private dirty = false;
  private readonly form: HTMLFormElement;
  private readonly question: HTMLInputElement;
  private readonly provenance: HTMLInputElement;
  private readonly message: HTMLElement;
  private readonly results: HTMLElement;
  private readonly submit: HTMLButtonElement;
  private readonly cancel: HTMLButtonElement;
  private readonly guidance: HTMLElement;

  constructor(readonly host: HTMLElement, private readonly activate: (document: LinkedLibraryDocument) => void | Promise<void>) {
    host.innerHTML = `<form class="librarian-form">
      <label>Find saved project knowledge<input name="question" type="search" maxlength="512" required autocomplete="off"></label>
      <label class="librarian-provenance"><input name="provenance" type="checkbox">Inspect recorded repository evidence</label>
      <div><button type="submit">Find sources</button><button type="button" data-cancel disabled>Cancel</button></div>
      </form><p data-message role="status"></p><p data-guidance></p><div class="librarian-results"></div>`;
    this.form = host.querySelector("form")!;
    this.question = host.querySelector('[name="question"]')!;
    this.provenance = host.querySelector('[name="provenance"]')!;
    this.message = host.querySelector("[data-message]")!;
    this.guidance = host.querySelector("[data-guidance]")!;
    this.results = host.querySelector(".librarian-results")!;
    this.submit = host.querySelector('[type="submit"]')!;
    this.cancel = host.querySelector("[data-cancel]")!;
    this.form.addEventListener("submit", event => { event.preventDefault(); void this.inspect(); });
    this.cancel.addEventListener("click", () => this.invalidate("Request cancelled. Find sources again to refresh."));
    for (const input of [this.question, this.provenance]) {
      input.addEventListener("input", () => this.invalidate("Query changed. Find sources to refresh."));
    }
    this.bind(null);
  }

  bind(binding: Binding | null): void {
    this.generation++;
    this.binding = binding;
    this.result = null;
    this.available = false;
    this.busy = false;
    this.opening = false;
    this.question.value = "";
    this.provenance.checked = false;
    this.results.replaceChildren();
    this.message.textContent = binding ? `Find saved sources in ${binding.project}.` : "Open a project to find saved sources.";
    this.controls();
  }

  focus(): void { this.question.focus(); }
  setDirty(dirty: boolean): void {
    this.dirty = dirty;
    if (dirty && this.opening) this.invalidate("Navigation cancelled by unsaved edits. Find sources again to refresh.");
    this.controls();
  }
  suspend(): void { this.invalidate("View closed. Find sources again to refresh."); }

  private invalidate(message: string): void {
    this.generation++;
    this.busy = false;
    this.opening = false;
    this.available = false;
    this.message.textContent = message;
    this.controls();
  }

  private controls(): void {
    this.submit.disabled = !this.binding || this.busy;
    this.cancel.disabled = !this.busy;
    this.host.setAttribute("aria-busy", String(this.busy));
    this.guidance.textContent = this.dirty
      ? "Unsaved changes: results describe saved files. Save or Discard before opening a source."
      : "Exact saved passages from this project. Literal retrieval can omit relevant notes; source freshness does not establish claim correctness.";
    for (const button of this.results.querySelectorAll<HTMLButtonElement>("button")) {
      button.disabled = !this.available || this.busy || this.dirty;
    }
  }

  private async inspect(): Promise<void> {
    const binding = this.binding;
    if (!binding || this.busy) return;
    const generation = ++this.generation;
    this.busy = true;
    this.available = false;
    this.message.textContent = `Checking saved sources in ${binding.project}…`;
    this.controls();
    try {
      const result = await inspectLibrarian(binding.root, binding.project, this.question.value, this.provenance.checked);
      if (generation !== this.generation || binding !== this.binding) return;
      if (result.packet.project !== binding.project || result.review.sources.project !== binding.project) {
        throw new Error("Source result belongs to another project.");
      }
      this.result = result;
      this.available = true;
      const review = result.review;
      this.message.textContent = `${review.sources.matches.length} passages / ${review.matched_notes} matched / ${review.scanned_notes} scanned / ${review.omitted_notes} omitted by retrieval limits.${review.sources.matches.length ? "" : " No saved source matches. Try specific names or words recorded in the notes."}`;
      this.results.replaceChildren(...review.sources.matches.map(passage => {
        const card = document.createElement("article");
        card.className = "librarian-source";
        const open = document.createElement("button");
        open.type = "button";
        open.textContent = `[${passage.citation}] Open ${passage.id}`;
        open.addEventListener("click", () => void this.open(result, passage.citation));
        card.append(open, text("p", `Lines ${passage.line_start}–${passage.line_end} / recorded status: ${passage.status ?? "not recorded"}${passage.partial_body ? " / partial note body" : ""}`), text("pre", passage.quote));
        if (passage.omitted_links) card.append(text("p", `${passage.omitted_links} recorded links omitted.`));
        const provenance = review.provenance?.notes.find(note => note.citation === passage.citation);
        if (provenance) card.append(renderProvenance(provenance));
        return card;
      }));
    } catch (error) {
      if (generation !== this.generation || binding !== this.binding) return;
      this.message.textContent = `Sources unavailable: ${message(error)}${this.result ? " Previous results are stale; find sources again." : ""}`;
    } finally {
      if (generation === this.generation && binding === this.binding) { this.busy = false; this.controls(); }
    }
  }

  private async open(result: DesktopLibrarianReview, citation: number): Promise<void> {
    const binding = this.binding;
    if (!binding || !this.available || this.busy || this.dirty || result !== this.result) return;
    const generation = ++this.generation;
    this.busy = true;
    this.opening = true;
    this.controls();
    try {
      const document = await followLibrarianSource(binding.root, binding.project, result.packet, citation);
      if (generation !== this.generation || binding !== this.binding || this.dirty) return;
      const source = result.packet.evidence.find(item => item.citation === citation);
      if (source?.id !== document.id) throw new Error("Loaded source identity does not match its citation.");
      await this.activate(document);
    } catch (error) {
      if (generation !== this.generation || binding !== this.binding) return;
      this.available = false;
      this.message.textContent = `Source unavailable: ${message(error)}. Find sources again to refresh.`;
    } finally {
      if (generation === this.generation && binding === this.binding) { this.busy = false; this.opening = false; this.controls(); }
    }
  }
}

function text(tag: string, value: string): HTMLElement {
  const node = document.createElement(tag);
  node.textContent = value;
  return node;
}

function renderProvenance(note: LibrarianNoteProvenance): HTMLElement {
  const details = document.createElement("details");
  details.append(text("summary", `Repository evidence: ${note.evidence_state} / ${note.claims.length} associated claims`));
  if (note.unmatched_claims) details.append(text("p", `${note.unmatched_claims} recorded claims outside this exact passage or ambiguously associated.`));
  for (const claim of note.claims) {
    details.append(text("p", `${claim.recorded_kind} / note line ${claim.note_line}: ${claim.claim}`));
    if (claim.rationale) details.append(text("p", claim.rationale));
    for (const source of claim.sources) {
      details.append(text("p", `${source.path}:${source.line_start ?? "?"}–${source.line_end ?? "?"} / ${source.freshness} / ${source.excerpt_state}`));
      if (source.excerpt !== null) details.append(text("pre", source.excerpt));
    }
  }
  return details;
}

function message(error: unknown): string {
  return typeof error === "object" && error !== null && "message" in error ? String(error.message) : String(error);
}
