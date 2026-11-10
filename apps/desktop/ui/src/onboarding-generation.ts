import { cancelLocalGeneration, generateOnboarding, localInferenceAvailability, reserveLocalGeneration } from "./api";
import type { GeneratedOnboarding, InferenceAvailability } from "./onboarding-types";
type Binding = { root: string; project: string };

/** Ephemeral controls for the native owned worker; no executable, URL or arbitrary model input. */
export class OnboardingGenerator {
  private binding: Binding | null = null;
  private packet = "";
  private generation = 0;
  private requestId: string | null = null;
  private running = false;
  private checking = false;
  private disabled = false;
  private availability: InferenceAvailability | null = null;
  private readonly status: HTMLElement;
  private readonly profile: HTMLSelectElement;
  constructor(private readonly host: HTMLElement, private readonly busy: (running: boolean) => void, private readonly generated: (result: GeneratedOnboarding) => void) {
    host.innerHTML = `<h3>2. Generate a local draft</h3>
      <p>Optional local Qwen3.5-9B generation. Inspect model setup, choose a device, then generate from the selected sources. Claims still need your source review.</p>
      <button type="button" data-inference-setup>Inspect model setup</button>
      <label>Device <select name="inference-profile"><option value="vulkan">GPU · automatic memory fitting</option><option value="cpu">CPU · slower</option></select></label>
      <button type="button" data-generate>Generate draft locally</button>
      <button type="button" data-cancel-generation>Cancel generation</button>
      <p role="status" data-generation-status>Local generation is off until explicitly requested. Existing captured plans can still be imported.</p>`;
    this.status = host.querySelector("[data-generation-status]")!;
    this.profile = host.querySelector('[name="inference-profile"]')!;
    host.querySelector("[data-inference-setup]")!.addEventListener("click", () => void this.inspect());
    host.querySelector("[data-generate]")!.addEventListener("click", () => void this.generate());
    host.querySelector("[data-cancel-generation]")!.addEventListener("click", () => this.cancel());
    this.profile.addEventListener("change", () => this.controls()); this.controls();
  }
  bind(binding: Binding | null): void { this.cancel(); this.binding = binding; this.packet = ""; this.controls(); }
  setPacket(packet: string): void { this.cancel(); this.packet = packet; this.controls(); }
  setDisabled(value: boolean): void { this.disabled = value; this.controls(); }
  cancel(): void {
    this.generation++;
    const id = this.requestId; this.requestId = null;
    if (id) void cancelLocalGeneration(id).catch(error => { this.status.textContent = `Cancellation could not be delivered: ${message(error)}. The native request deadline remains active.`; });
    if (this.running) { this.running = false; this.busy(false); this.status.textContent = "Generation cancelled. The native worker is being stopped; a new request waits for cleanup."; }
    this.checking = false; this.controls();
  }
  private controls(): void {
    const available = this.availability;
    const installed = available?.supported && available.model_present && (this.profile.value === "cpu" ? available.cpu_present : available.vulkan_present);
    this.host.querySelector<HTMLButtonElement>("[data-inference-setup]")!.disabled = this.running || this.checking || this.disabled;
    this.host.querySelector<HTMLButtonElement>("[data-generate]")!.disabled = !this.binding || !this.packet || !installed || this.running || this.disabled || this.checking;
    this.host.querySelector<HTMLButtonElement>("[data-cancel-generation]")!.disabled = !this.running;
    this.profile.disabled = this.running || this.disabled || this.checking;
    this.host.setAttribute("aria-busy", String(this.running || this.checking));
  }
  private async inspect(): Promise<void> {
    if (this.running || this.checking || this.disabled) return;
    const generation = ++this.generation; this.checking = true; this.controls();
    try {
      const result = await localInferenceAvailability(); if (generation !== this.generation) return;
      this.availability = result;
      this.status.textContent = result.supported
        ? `${result.model}. Model ${result.model_present ? "present" : "missing"}; GPU runtime ${result.vulkan_present ? "present" : "missing"}; CPU runtime ${result.cpu_present ? "present" : "missing"}. Location: ${result.location}. Exact SHA-256 checks run before generation. Nothing is downloaded.`
        : "Native local generation currently supports Linux. Import a captured plan to continue reviewing here.";
    } catch (error) { if (generation === this.generation) this.status.textContent = `Model setup unavailable: ${message(error)}`; }
    finally { if (generation === this.generation) { this.checking = false; this.controls(); } }
  }
  private async generate(): Promise<void> {
    const binding = this.binding, packet = this.packet;
    if (!binding || !packet || this.running || this.disabled || this.checking) return;
    const generation = ++this.generation; this.running = true; this.busy(true); this.controls();
    this.status.textContent = "Verifying Qwen9B and the pinned runtime, then generating source-backed draft claims…";
    let id: string | null = null;
    try {
      id = await reserveLocalGeneration();
      if (generation !== this.generation || binding !== this.binding) { await cancelLocalGeneration(id); return; }
      this.requestId = id;
      const result = await generateOnboarding(id, binding.root, binding.project, packet, this.profile.value as "vulkan" | "cpu");
      if (generation !== this.generation || binding !== this.binding) return;
      if (result.review.draft.preview.root !== binding.root || result.review.draft.preview.project !== binding.project) throw new Error("Generated draft belongs to another project or root.");
      this.generated(result);
      this.status.textContent = `Generated ${result.metrics.length} source groups with Qwen9B. Worker stopped. Review every claim and its cited code before previewing and approving publication.`;
    } catch (error) { if (generation === this.generation) this.status.textContent = `Generation unavailable: ${message(error)}`; }
    finally {
      if (id) await cancelLocalGeneration(id).catch(() => {});
      if (generation === this.generation) { this.requestId = null; this.running = false; this.busy(false); this.controls(); }
    }
  }
}
function message(error: unknown): string { return typeof error === "object" && error !== null && "message" in error ? String(error.message) : String(error); }
