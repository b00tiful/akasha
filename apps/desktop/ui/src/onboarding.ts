import { publishOnboarding, reviewOnboarding } from "./api";
import type { OnboardingPlan, OnboardingResult, OnboardingReview } from "./onboarding-types";
import { OnboardingSourcePicker } from "./onboarding-sources";

type Binding = { root: string; project: string };

/** In-memory claim editor. All parsing of product evidence, checks and mutations stay in Rust. */
export class OnboardingPanel {
  private binding: Binding | null = null;
  private generation = 0;
  private busy = false;
  private publishing = false;
  private dirty = false;
  private packetJson = "";
  private preparedPacketJson = "";
  private readonly sources: OnboardingSourcePicker;
  private plan: OnboardingPlan | null = null;
  private review: OnboardingReview | null = null;
  private approvedPlan: string | null = null;
  private readonly packetInput: HTMLInputElement;
  private readonly planInput: HTMLInputElement;
  private readonly inspect: HTMLButtonElement;
  private readonly publish: HTMLButtonElement;
  private readonly approval: HTMLInputElement;
  private readonly message: HTMLElement;
  private readonly editor: HTMLElement;
  private readonly preview: HTMLElement;

  constructor(private readonly host: HTMLElement, private readonly published: (result: OnboardingResult) => Promise<void>) {
    host.innerHTML = `<div data-source-picker></div><h3>3. Review and publish a draft</h3>
      <p>Choose plan.json generated from the selected sources above, or import both files from an earlier capture. Review claims beside captured code, then preview and approve the exact changes. Notes retain their recorded draft metadata. Corrections stay in memory until the library is reloaded or changed.</p>
      <label>Earlier source packet (optional when sources are selected above) <input name="packet" type="file" accept=".json,application/json"></label>
      <label>Claim plan <input name="plan" type="file" accept=".json,application/json"></label>
      <button type="button" data-preview>Load / preview changes</button>
      <p data-dirty hidden>Unsaved note changes: Save or Discard before publishing reviewed onboarding.</p><p role="status" data-status></p><div data-editor></div><div data-proposal></div>
      <label>Paste the full preview ID to approve this publication <input name="approval" type="text" autocomplete="off" spellcheck="false"></label>
      <button type="button" data-publish>Publish reviewed notes</button>`;
    this.packetInput = host.querySelector('[name="packet"]')!;
    this.planInput = host.querySelector('[name="plan"]')!;
    this.inspect = host.querySelector('[data-preview]')!;
    this.publish = host.querySelector('[data-publish]')!;
    this.approval = host.querySelector('[name="approval"]')!;
    this.message = host.querySelector('[data-status]')!;
    this.editor = host.querySelector('[data-editor]')!;
    this.preview = host.querySelector('[data-proposal]')!;
    this.sources = new OnboardingSourcePicker(host.querySelector('[data-source-picker]')!, result => {
      this.generation++; this.busy = false; this.plan = null; this.review = null;
      this.preparedPacketJson = result?.packet_json ?? ""; this.packetJson = this.preparedPacketJson;
      this.packetInput.value = ""; this.planInput.value = ""; this.editor.replaceChildren();
      this.invalidate(result ? "Selected source packet retained. Choose its generated plan.json to review." : "Source selection changed. Prepare fresh sources and a matching draft.");
    });
    for (const input of [this.packetInput, this.planInput]) input.addEventListener("change", () => {
      if (this.publishing) return;
      if (input === this.packetInput) { this.preparedPacketJson = ""; this.sources.clear(); }
      else this.sources.suspend();
      this.generation++; this.busy = false; this.plan = null; this.review = null; this.packetJson = this.preparedPacketJson;
      this.editor.replaceChildren(); this.invalidate("Inputs changed. Load a fresh source review.");
    });
    this.inspect.addEventListener("click", () => void this.load());
    this.approval.addEventListener("input", () => this.controls());
    this.publish.addEventListener("click", () => void this.apply());
    this.controls();
  }

  get isPublishing(): boolean { return this.publishing; }
  bind(binding: Binding | null): void {
    if (this.publishing) return; // Shell prevents project changes until publication completes.
    if (this.binding?.root === binding?.root && this.binding?.project === binding?.project) return;
    this.binding = binding; this.generation++; this.busy = false;
    this.packetJson = ""; this.preparedPacketJson = ""; this.plan = null; this.review = null;
    this.sources.bind(binding);
    this.packetInput.value = ""; this.planInput.value = "";
    this.editor.replaceChildren();
    this.invalidate(binding ? `Review captured onboarding for ${binding.project}.` : "Load a project first.");
  }
  setDirty(dirty: boolean): void {
    this.dirty = dirty;
    if (dirty && !this.publishing) { this.generation++; this.busy = false; this.invalidate("Save or Discard editor changes before publishing. Preview again afterwards."); }
    this.controls();
  }
  suspend(): boolean {
    if (this.publishing) return false;
    this.generation++; this.busy = false;
    this.sources.suspend();
    this.invalidate("Review closed. Corrections remain in memory; preview again before publishing.");
    return true;
  }
  focus(): void { this.sources.focus(); }
  private invalidate(message: string): void {
    this.approvedPlan = null; this.approval.value = ""; this.preview.replaceChildren();
    this.message.textContent = message; this.controls();
  }
  private controls(): void {
    this.sources?.setDisabled(this.busy);
    this.inspect.disabled = !this.binding || this.busy;
    for (const input of [this.packetInput, this.planInput]) input.disabled = this.busy;
    this.approval.disabled = this.busy || !this.approvedPlan;
    this.publish.disabled = this.busy || this.dirty || !this.approvedPlan
      || this.approval.value !== this.review?.draft.preview.preview_id;
    this.host.setAttribute("aria-busy", String(this.busy));
    this.host.querySelector<HTMLElement>("[data-dirty]")!.hidden = !this.dirty;
    for (const input of this.editor.querySelectorAll<HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement | HTMLButtonElement>("input, textarea, select, button")) {
      input.disabled = this.busy || input.dataset.limit === "true";
    }
  }
  private async read(input: HTMLInputElement): Promise<string> {
    const file = input.files?.[0];
    if (!file || file.size > 16_000) throw new Error("Choose a JSON file of at most 16000 bytes for each input.");
    return file.text();
  }
  private async load(): Promise<void> {
    const binding = this.binding;
    if (!binding || this.busy) return;
    this.sources.suspend();
    const generation = ++this.generation;
    this.busy = true; this.invalidate("Rechecking captured sources and current project state…");
    try {
      const packet = this.plan ? this.packetJson : this.preparedPacketJson || await this.read(this.packetInput);
      const planJson = this.plan ? JSON.stringify(this.plan) : await this.read(this.planInput);
      if (generation !== this.generation || binding !== this.binding) return;
      const result = await reviewOnboarding(binding.root, binding.project, packet, planJson);
      if (generation !== this.generation || binding !== this.binding) return;
      if (result.draft.preview.root !== binding.root || result.draft.preview.project !== binding.project) throw new Error("Review belongs to another project or root.");
      this.packetJson = packet; this.plan = JSON.parse(planJson) as OnboardingPlan; this.review = result;
      this.renderEditor();
      this.approvedPlan = planJson;
      this.renderPreview(result);
      this.message.textContent = "Fresh preview ready. Check claims, exact proposed Markdown and affected paths before approving. Source identity does not certify meaning.";
    } catch (error) {
      if (generation === this.generation && binding === this.binding) this.invalidate(`Review unavailable: ${errorMessage(error)}`);
    } finally { if (generation === this.generation) { this.busy = false; this.controls(); } }
  }
  private renderEditor(): void {
    this.editor.replaceChildren();
    const plan = this.plan!, review = this.review!;
    const change = () => this.invalidate("Corrections changed. Preview again to refresh evidence checks and approval.");
    for (const [index, note] of plan.notes.entries()) {
      const section = element("section", undefined, "onboarding-note");
      const slot = review.identities?.slots.find(slot => slot.slot === note.slot);
      // Core canonicalizes slotted notes by slot; the imported plan can be in any order.
      const reviewIndex = slot ? review.identities!.slots.indexOf(slot) : index;
      const reviewed = review.draft.review.notes[reviewIndex]!;
      section.append(element("h3", reviewed.path), disclosure("Recorded metadata", JSON.stringify(reviewed.fields, null, 2)));
      const excerpts = review.excerpts.filter(excerpt => !slot || slot.sources.includes(excerpt.source));
      const browse = element("details"); browse.append(element("summary", "Browse captured source excerpts"));
      for (const excerpt of excerpts) {
        const details = disclosure(`${excerpt.path}:${excerpt.line_start}–${excerpt.line_end}`, excerpt.text);
        const add = button("Add claim citing this excerpt", () => {
          if (note.evidence.length >= 4) return;
          note.evidence.push({ kind: "fact", claim: "", sources: [{ source: excerpt.source, excerpt: excerpt.excerpt }] });
          change(); this.renderEditor();
        });
        add.dataset.limit = String(note.evidence.length >= 4); details.append(add); browse.append(details);
      }
      section.append(browse);
      for (const [claimIndex, claim] of note.evidence.entries()) {
        const card = element("article", undefined, "onboarding-claim");
        const wording = document.createElement("textarea"); wording.value = claim.claim; wording.rows = 3;
        wording.setAttribute("aria-label", `Note ${index + 1} claim ${claimIndex + 1}`);
        wording.addEventListener("input", () => { claim.claim = wording.value; change(); });
        const kind = document.createElement("select"); kind.setAttribute("aria-label", "Evidence kind");
        for (const value of ["fact", "inference", "unknown"]) { const option = document.createElement("option"); option.value = value; option.textContent = value; kind.append(option); }
        kind.value = claim.kind;
        kind.addEventListener("change", () => { claim.kind = kind.value; if (claim.kind === "unknown") claim.sources = []; change(); this.renderEditor(); });
        const rationale = document.createElement("textarea"); rationale.value = claim.rationale ?? ""; rationale.rows = 2; rationale.setAttribute("aria-label", "Rationale");
        rationale.addEventListener("input", () => { if (rationale.value) claim.rationale = rationale.value; else delete claim.rationale; change(); });
        const remove = button("Remove claim", () => { if (note.evidence.length <= 1) return; note.evidence.splice(claimIndex, 1); change(); this.renderEditor(); });
        remove.dataset.limit = String(note.evidence.length <= 1);
        card.append(element("h4", `Claim ${claimIndex + 1}`), wording, kind, element("p", "Rationale (required for inference / unknown)"), rationale, remove);
        for (const excerpt of excerpts) {
          const label = document.createElement("label");
          const select = document.createElement("input"); select.type = "checkbox";
          select.checked = claim.sources.some(source => source.source === excerpt.source && source.excerpt === excerpt.excerpt);
          select.dataset.limit = String(claim.kind === "unknown");
          select.addEventListener("change", () => {
            claim.sources = claim.sources.filter(source => source.source !== excerpt.source || source.excerpt !== excerpt.excerpt);
            if (select.checked) claim.sources.push({ source: excerpt.source, excerpt: excerpt.excerpt }); change();
          });
          label.append(select, document.createTextNode(` ${excerpt.path}:${excerpt.line_start}–${excerpt.line_end}`));
          card.append(label);
          if (select.checked) card.append(disclosure("Selected exact source", excerpt.text));
        }
        section.append(card);
      }
      this.editor.append(section);
    }
    this.controls();
  }
  private renderPreview(review: OnboardingReview): void {
    const { draft } = review;
    this.preview.append(element("h3", "Exact publication preview"), element("p", `Project: ${draft.preview.project} / root: ${draft.preview.root}`), element("pre", draft.preview.preview_id));
    for (const note of draft.preview.notes) this.preview.append(element("p", `${note.action}: ${note.path} / ${note.evidence_claims} claims`));
    this.preview.append(element("p", `Update index: ${draft.preview.index_changed}; roadmap: ${draft.preview.roadmap_changed}; state: ${draft.preview.state_changed}`));
    for (const note of draft.proposal.notes) this.preview.append(disclosure(`Exact proposed ${note.path}`, note.source));
    this.preview.append(disclosure("Exact proposed index", draft.proposal.index), disclosure("Exact proposed roadmap", draft.proposal.roadmap),
      disclosure("Coverage and sampling gaps", JSON.stringify({ uncited_sources: draft.review.uncited_sources, sampled_sources: draft.review.sampled_sources, uncited_focus_excerpts: draft.review.uncited_focus_excerpts ?? [] }, null, 2)));
  }
  private async apply(): Promise<void> {
    const binding = this.binding, planJson = this.approvedPlan, previewId = this.review?.draft.preview.preview_id;
    if (!binding || this.busy || this.dirty || !planJson || !previewId || this.approval.value !== previewId) return;
    this.busy = true; this.publishing = true; this.approval.value = ""; this.controls();
    this.message.textContent = "Publishing the explicitly approved preview. Wait for the result; closing cannot undo committed changes.";
    try {
      const result = await publishOnboarding(binding.root, binding.project, this.packetJson, planJson, previewId);
      this.publishing = false; this.busy = false;
      this.plan = null; this.review = null; this.packetJson = ""; this.preparedPacketJson = ""; this.sources.clear();
      this.editor.replaceChildren(); this.packetInput.value = ""; this.planInput.value = "";
      this.invalidate(`Published ${result.created_notes.length} notes. Reloading the library…`);
      try { await this.published(result); }
      catch (error) { this.message.textContent = `Publication succeeded. Library refresh failed: ${errorMessage(error)}. Reload the library before continuing.`; }
    } catch (error) {
      this.publishing = false; this.busy = false;
      this.invalidate(`Publication unavailable: ${errorMessage(error)}. No automatic retry. Inspect current state before preparing a fresh review.`);
    } finally { this.controls(); }
  }
}
function errorMessage(error: unknown): string { return typeof error === "object" && error !== null && "message" in error ? String(error.message) : String(error); }
function element(tag: string, text?: string, className?: string): HTMLElement {
  const node = document.createElement(tag); if (text !== undefined) node.textContent = text; if (className) node.className = className; return node;
}
function disclosure(title: string, content: string): HTMLElement { const node = element("details"); node.append(element("summary", title), element("pre", content)); return node; }
function button(title: string, action: () => void): HTMLButtonElement { const node = document.createElement("button"); node.type = "button"; node.textContent = title; node.addEventListener("click", action); return node; }
