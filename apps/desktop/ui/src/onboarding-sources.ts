import { prepareOnboardingSources } from "./api";
import type { OnboardingSelection, OnboardingSources } from "./onboarding-types";

type Binding = { root: string; project: string };

/** Source choices are presentation state. Rust owns scanning, excerpts and packet identities. */
export class OnboardingSourcePicker {
  private binding: Binding | null = null;
  private generation = 0;
  private busy = false;
  private disabled = false;
  private readonly paths: HTMLTextAreaElement;
  private readonly focusTerms: HTMLTextAreaElement;
  private readonly sampling: HTMLInputElement;
  private readonly mode: HTMLSelectElement;
  private readonly status: HTMLElement;
  private readonly output: HTMLElement;

  constructor(private readonly host: HTMLElement, private readonly replaced: (sources: OnboardingSources | null) => void) {
    host.innerHTML = `<h3>1. Choose repository sources</h3>
      <p>Paths are relative to this project's registered repository. Discovery reports its bounded selection and gaps. Adjust the explicit file list to review different sources.</p>
      <label>Selection <select name="source-mode"><option value="discover">Discover within directories</option><option value="files">Choose exact files</option></select></label>
      <label>Directories or files, one per line <textarea name="source-paths" rows="3" placeholder="src&#10;docs&#10;tests"></textarea></label>
      <label>Optional literal focus terms, one per line <textarea name="source-focus" rows="2"></textarea></label>
      <label><input name="source-sampling" type="checkbox"> Allow partial windows from large files</label>
      <button type="button" data-prepare-sources>Inspect selected sources</button>
      <button type="button" data-cancel-sources disabled>Cancel source inspection</button>
      <p role="status" data-source-status>Choose directories or files to begin. Nothing is scanned automatically.</p><div data-source-output></div>`;
    this.paths = host.querySelector('[name="source-paths"]')!;
    this.focusTerms = host.querySelector('[name="source-focus"]')!;
    this.sampling = host.querySelector('[name="source-sampling"]')!;
    this.mode = host.querySelector('[name="source-mode"]')!;
    this.status = host.querySelector('[data-source-status]')!;
    this.output = host.querySelector('[data-source-output]')!;
    for (const input of [this.paths, this.focusTerms, this.sampling, this.mode]) input.addEventListener("input", () => {
      if (this.disabled) return;
      this.clear(); this.replaced(null); this.status.textContent = "Selection changed. Inspect again before generating a draft.";
    });
    host.querySelector('[data-prepare-sources]')!.addEventListener("click", () => void this.prepare());
    host.querySelector('[data-cancel-sources]')!.addEventListener("click", () => this.suspend());
    this.controls();
  }
  bind(binding: Binding | null): void {
    if (this.binding?.root === binding?.root && this.binding?.project === binding?.project) return;
    this.binding = binding; this.clear(); this.paths.value = ""; this.focusTerms.value = "";
    this.sampling.checked = false; this.mode.value = "discover";
    this.status.textContent = "Choose directories or files to begin. Nothing is scanned automatically.";
  }
  setDisabled(disabled: boolean): void { this.disabled = disabled; this.controls(); }
  focus(): void { this.paths.focus(); }
  clear(): void { this.generation++; this.busy = false; this.output.replaceChildren(); this.controls(); }
  suspend(): void {
    const pending = this.busy;
    this.generation++; this.busy = false; this.controls();
    if (pending) this.status.textContent = "Source inspection stopped. A running bounded read may finish; its result will be ignored.";
  }
  private controls(): void {
    for (const input of this.host.querySelectorAll<HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement | HTMLButtonElement>("input, textarea, select, button")) input.disabled = this.disabled || !this.binding || this.busy;
    this.host.querySelector<HTMLButtonElement>("[data-cancel-sources]")!.disabled = !this.busy || this.disabled;
    this.host.setAttribute("aria-busy", String(this.busy));
  }
  private async prepare(): Promise<void> {
    const binding = this.binding;
    if (!binding || this.busy || this.disabled) return;
    const lines = (value: string) => value.split(/\r?\n/u).map(line => line.trim()).filter(Boolean);
    const selection: OnboardingSelection = { mode: this.mode.value as OnboardingSelection["mode"], paths: lines(this.paths.value), focus: lines(this.focusTerms.value), sample_large_files: this.sampling.checked };
    this.replaced(null); this.clear();
    const generation = ++this.generation; this.busy = true; this.controls();
    this.status.textContent = "Inspecting bounded sources and rebuilding their exact identities…";
    try {
      const result = await prepareOnboardingSources(binding.root, binding.project, selection);
      if (generation !== this.generation || binding !== this.binding) return;
      if (result.root !== binding.root || result.project !== binding.project) throw new Error("Sources belong to another project or root.");
      this.render(result); this.replaced(result);
      this.status.textContent = "Sources ready. Generate a local draft below or import the plan for this exact packet. A new source selection clears the old draft.";
    } catch (error) {
      if (generation === this.generation) this.status.textContent = `Source inspection unavailable: ${message(error)}`;
    } finally { if (generation === this.generation) { this.busy = false; this.controls(); } }
  }
  private render(result: OnboardingSources): void {
    const paths = [...new Set(result.excerpts.map(excerpt => excerpt.path))];
    this.output.append(node("h4", `${paths.length} selected files · ${result.identities.slots.length} planned notes`));
    const useFiles = node("button", "Use selected paths as an editable file list") as HTMLButtonElement;
    useFiles.type = "button"; useFiles.addEventListener("click", () => {
      this.mode.value = "files"; this.paths.value = paths.join("\n"); this.clear(); this.replaced(null);
      this.status.textContent = "Edit the exact file list, then inspect again. Prior discovery coverage no longer describes this selection.";
      this.paths.focus();
    });
    this.output.append(useFiles);
    if (result.coverage) {
      const coverage = result.coverage;
      this.output.append(node("p", `${coverage.selected.length}/${coverage.candidates} candidates selected; ${coverage.omitted.length} omitted; ${coverage.excluded_entries} entries excluded.`));
      for (const group of coverage.implementation_groups) this.output.append(node("p", `${group.selected ? "Represented" : "Missing"} implementation group: ${group.directory} (${group.selected}/${group.candidates}, filename heuristic)`));
      this.output.append(details("Discovery scope and all reported gaps", JSON.stringify(coverage, null, 2)));
    } else this.output.append(node("p", "Explicit file selection. No directory inventory or repository coverage claim."));
    this.output.append(details("Planned note identities and unresolved metadata", JSON.stringify(result.identities, null, 2)), details("Partial-file coverage", JSON.stringify(result.sampled_sources, null, 2)));
    for (const excerpt of result.excerpts) this.output.append(details(`${excerpt.path}:${excerpt.line_start}–${excerpt.line_end}`, excerpt.text));
    this.output.append(node("h4", "2. Generate a local draft"), node("p", "Generation currently uses the local command-line runner. Save the exact packet below as packet.json, run the command with your existing local model settings, then select its generated plan.json below. This review keeps the selected packet in memory."), node("pre", "node scripts/local-onboarding.mjs --root <root> --project <project> --packet packet.json --save-inputs <new-private-directory> --server <llama-server> --model <GGUF> --model-sha256 <SHA256> --gpu-layers <0-or-auto> --worker-session"));
    const label = node("label", "Exact source packet (select and copy to packet.json)");
    const packet = document.createElement("textarea"); packet.readOnly = true; packet.rows = 5; packet.value = result.packet_json; packet.setAttribute("aria-label", "Exact source packet");
    label.append(packet); this.output.append(label);
    const select = node("button", "Select packet text") as HTMLButtonElement;
    select.type = "button"; select.addEventListener("click", () => { packet.focus(); packet.select(); }); this.output.append(select);
  }
}
function node(tag: string, text: string): HTMLElement { const value = document.createElement(tag); value.textContent = text; return value; }
function details(title: string, content: string): HTMLElement { const value = document.createElement("details"); value.append(node("summary", title), node("pre", content)); return value; }
function message(error: unknown): string { return typeof error === "object" && error !== null && "message" in error ? String(error.message) : String(error); }
