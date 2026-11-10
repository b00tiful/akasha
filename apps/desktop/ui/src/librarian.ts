import { inspectLibrarian, followLibrarianSource, assistLibrarian, localInferenceAvailability, reserveLocalGeneration, cancelLocalGeneration } from "./api";
import type { DesktopLibrarianReview, LibrarianNoteProvenance } from "./librarian-types";
import type { InferenceAvailability } from "./onboarding-types";
import type { LinkedLibraryDocument } from "./types";

interface Binding { root: string; project: string }

/** Ephemeral presentation and obsolete-request suppression; Rust owns every source check. */
export class LibrarianPanel {
  private binding: Binding | null = null;
  private generation = 0;
  private result: DesktopLibrarianReview[] = [];
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
  private requestId: string | null = null;
  private availability: InferenceAvailability | null = null;
  private readonly profile: HTMLSelectElement;
  private readonly assist: HTMLButtonElement;
  private readonly setup: HTMLButtonElement;
  private readonly modelStatus: HTMLElement;
  private readonly terms: HTMLTextAreaElement;

  constructor(readonly host: HTMLElement, private readonly activate: (document: LinkedLibraryDocument) => void | Promise<void>) {
    host.innerHTML = `<form class="librarian-form">
      <label>Find saved project knowledge<input name="question" type="search" maxlength="512" required autocomplete="off"></label>
      <label class="librarian-provenance"><input name="provenance" type="checkbox">Inspect recorded repository evidence</label>
      <div><button type="submit">Find sources</button><button type="button" data-cancel disabled>Cancel</button></div>
      <details><summary>Optional local search assistance</summary>
      <p>Qwen3.5-9B suggests search words from your question only. Results are saved passages to review. Questions up to 160 characters; no answer is generated.</p>
      <button type="button" data-model-setup>Inspect model setup</button>
      <label>Device <select name="librarian-profile"><option value="vulkan">GPU · automatic memory fitting</option><option value="cpu">CPU · slower</option></select></label>
      <button type="button" data-assisted>Find with local model</button><p data-model-status role="status">Off until explicitly requested. Nothing is downloaded.</p>
      <label>Refine search words<textarea name="librarian-terms" rows="3" maxlength="1100" placeholder="One literal word or phrase per line"></textarea></label>
      <p>Optional: edit suggested words, then choose Find sources. Up to eight phrases of 128 characters. This searches these words with your original question retained, without running the model. Clear to restore question matching. Local assistance always starts from the question alone.</p></details>
      </form><p data-message role="status"></p><p data-guidance></p><div class="librarian-results"></div>`;
    this.form = host.querySelector("form")!;
    this.question = host.querySelector('[name="question"]')!;
    this.provenance = host.querySelector('[name="provenance"]')!;
    this.message = host.querySelector("[data-message]")!;
    this.guidance = host.querySelector("[data-guidance]")!;
    this.results = host.querySelector(".librarian-results")!;
    this.submit = host.querySelector('[type="submit"]')!;
    this.cancel = host.querySelector("[data-cancel]")!;
    this.profile = host.querySelector('[name="librarian-profile"]')!;
    this.assist = host.querySelector("[data-assisted]")!;
    this.setup = host.querySelector("[data-model-setup]")!;
    this.modelStatus = host.querySelector("[data-model-status]")!;
    this.terms = host.querySelector('[name="librarian-terms"]')!;
    this.setup.addEventListener("click", () => void this.inspectSetup());
    this.assist.addEventListener("click", () => void this.inspect(true));
    this.form.addEventListener("submit", event => { event.preventDefault(); void this.inspect(); });
    this.cancel.addEventListener("click", () => this.invalidate("Request cancelled. Find sources again to refresh."));
    for (const input of [this.question, this.provenance, this.profile, this.terms]) {
      input.addEventListener("input", () => this.invalidate("Query changed. Find sources to refresh."));
    }
    this.bind(null);
  }

  bind(binding: Binding | null): void {
    this.invalidate("Project changed.");
    this.binding = binding;
    this.result = [];
    this.available = false;
    this.busy = false;
    this.opening = false;
    this.question.value = "";
    this.provenance.checked = false;
    this.profile.value = "vulkan";
    this.terms.value = "";
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
    const id = this.requestId;
    this.requestId = null;
    if (id) {
      const generation = this.generation;
      void cancelLocalGeneration(id).catch(error => {
        if (generation === this.generation) this.message.textContent = `Cancellation could not be delivered: ${String(error)}. The native deadline remains active.`;
      });
    }
    this.busy = false;
    this.opening = false;
    this.available = false;
    this.message.textContent = message;
    this.controls();
  }

  private controls(): void {
    this.submit.disabled = !this.binding || this.busy;
    this.cancel.disabled = !this.busy;
    const available = this.availability;
    const installed = available?.supported && available.model_present && (this.profile.value === "cpu" ? available.cpu_present : available.vulkan_present);
    this.assist.disabled = !this.binding || this.busy || !installed || !this.question.value.trim() || [...this.question.value].length > 160;
    this.setup.disabled = this.busy;
    this.host.setAttribute("aria-busy", String(this.busy));
    this.guidance.textContent = this.dirty
      ? "Unsaved changes: results describe saved files. Save or Discard before opening a source."
      : "Exact saved passages from this project. Literal retrieval can omit relevant notes; source freshness does not establish claim correctness.";
    for (const button of this.results.querySelectorAll<HTMLButtonElement>("button")) {
      button.disabled = !this.available || this.busy || this.dirty;
    }
  }

  private async inspectSetup(): Promise<void> {
    if (this.busy) return;
    const generation = ++this.generation;
    this.busy = true; this.controls();
    try {
      const available = await localInferenceAvailability();
      if (generation !== this.generation) return;
      this.availability = available;
      this.modelStatus.textContent = available.supported
        ? `${available.model}. Model ${available.model_present ? "present" : "missing"}; GPU runtime ${available.vulkan_present ? "present" : "missing"}; CPU runtime ${available.cpu_present ? "present" : "missing"}. ${available.location}. Full artifact verification runs on request.`
        : "Local assistance currently supports Linux. Find sources remains available.";
    } catch (error) { if (generation === this.generation) this.modelStatus.textContent = `Model setup unavailable: ${message(error)}`; }
    finally { if (generation === this.generation) { this.busy = false; this.controls(); } }
  }

  private async inspect(assisted = false): Promise<void> {
    const binding = this.binding;
    if (!binding || this.busy) return;
    const generation = ++this.generation;
    this.busy = true;
    this.available = false;
    this.message.textContent = `Checking saved sources in ${binding.project}…`;
    this.controls();
    let id: string | null = null;
    try {
      const question = this.question.value, provenance = this.provenance.checked;
      const profile = this.profile.value as "vulkan" | "cpu";
      let packets: DesktopLibrarianReview[], terms: string[] = [];
      if (assisted) {
        id = await reserveLocalGeneration();
        if (generation !== this.generation || binding !== this.binding) { await cancelLocalGeneration(id); return; }
        this.requestId = id;
        this.message.textContent = "Verifying Qwen9B, suggesting search words, then checking saved sources…";
        const result = await assistLibrarian(id, binding.root, binding.project, question, provenance, profile);
        packets = [result.initial, ...(result.expanded ? [result.expanded] : [])];
        terms = result.terms;
      } else {
        const terms = this.terms.value.split(/\r?\n/u).map(term => term.trim()).filter(Boolean);
        if (terms.length > 8 || terms.some(term => [...term].length > 128)) throw new Error("Use at most eight search phrases of 128 characters.");
        packets = [await (terms.length ? inspectLibrarian(binding.root, binding.project, question, provenance, terms) : inspectLibrarian(binding.root, binding.project, question, provenance))];
      }
      if (generation !== this.generation || binding !== this.binding) return;
      if (packets.some(result => result.packet.project !== binding.project || result.review.sources.project !== binding.project || result.packet.question !== question.trim())) {
        throw new Error("Source result belongs to another project.");
      }
      this.result = packets;
      if (assisted) this.terms.value = terms.join("\n");
      this.available = true;
      const review = packets[0]!.review;
      this.message.textContent = `${review.sources.matches.length} passages / ${review.matched_notes} matched / ${review.scanned_notes} scanned / ${review.omitted_notes} omitted by retrieval limits.${review.sources.matches.length ? "" : " No saved source matches. Try specific names or words recorded in the notes."}`;
      if (assisted) this.message.textContent = `Literal: ${this.message.textContent} Local search words: ${terms.join(", ") || "none; literal results retained"}. Worker stopped. Matches do not establish an answer or prove that knowledge is absent.`;
      const seen = new Set<string>();
      this.results.replaceChildren(...packets.map((result, index) => {
        const section = document.createElement("section");
        if (assisted) section.append(text("h3", index === 0 ? "Original search" : "Additional local search"));
        if (index) section.append(text("p", `${result.review.matched_notes} matched / ${result.review.scanned_notes} scanned / ${result.review.omitted_notes} omitted by retrieval limits. Exact duplicate passages shown above.`));
        section.append(...result.review.sources.matches.filter(passage => {
          const key = JSON.stringify([passage.id, passage.fingerprint, passage.line_start, passage.line_end, passage.quote]);
          if (seen.has(key)) return false;
          seen.add(key); return true;
        }).map(passage => {
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
        return section;
      }));
    } catch (error) {
      if (generation !== this.generation || binding !== this.binding) return;
      this.message.textContent = `Sources unavailable: ${message(error)}${this.result.length ? " Previous results are stale; find sources again." : ""}`;
    } finally {
      if (id) await cancelLocalGeneration(id).catch(() => {});
      if (generation === this.generation && binding === this.binding) { this.requestId = null; this.busy = false; this.controls(); }
    }
  }

  private async open(result: DesktopLibrarianReview, citation: number): Promise<void> {
    const binding = this.binding;
    if (!binding || !this.available || this.busy || this.dirty || !this.result.includes(result)) return;
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
