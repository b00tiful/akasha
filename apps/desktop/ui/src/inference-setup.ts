import { cancelLocalGeneration, importInferenceAsset, localInferenceAvailability, reserveLocalGeneration, verifyInferenceAsset } from "./api";
import type { InferenceAvailability } from "./onboarding-types";

export type InferenceAsset = "model" | "vulkan" | "cpu";
export interface InferenceSetupResult { asset: InferenceAsset; outcome: "installed" | "already-installed" | "verified"; bytes: number; location: string }

/** Shared explicit offline setup; native Rust owns paths, pins, staging and publication. */
export class LocalInferenceSetup {
  private generation = 0;
  private requestId: string | null = null;
  private running = false;
  private disabled = false;
  private readonly source: HTMLInputElement;
  private readonly asset: HTMLSelectElement;
  private readonly status: HTMLElement;
  constructor(private readonly host: HTMLElement, private readonly changed: (availability: InferenceAvailability) => void, private readonly busy: (running: boolean) => void) {
    host.innerHTML = `<details><summary>Install or verify local model files</summary>
      <p>Offline setup for Linux x86-64. Import the selected Qwen3.5-9B UD-Q4_K_XL GGUF or an extracted llama.cpp b12110 runtime folder. Model and runtime are installed separately. Allow about 6 GB for the model plus runtime space.</p>
      <label>Artifact <select name="setup-asset"><option value="model">Qwen3.5-9B model</option><option value="vulkan">GPU runtime · b12110 Vulkan</option><option value="cpu">CPU runtime · b12110</option></select></label>
      <label>Local source path <input name="setup-source" type="text" maxlength="4096" autocomplete="off" placeholder="/absolute/path/to/model.gguf"></label>
      <p data-setup-source-help></p>
      <button type="button" data-setup-import>Import verified files</button>
      <button type="button" data-setup-verify>Verify installed artifact</button>
      <button type="button" data-setup-cancel>Cancel setup</button>
      <p>Files are copied to Akasha’s local inference folder after exact identity checks. Existing installations are preserved. Nothing is downloaded or executed during setup. Generation still checks all files each time.</p>
      <p data-setup-status role="status">Choose a local file or extracted runtime folder to install, or verify an installed artifact.</p></details>`;
    this.source = host.querySelector('[name="setup-source"]')!;
    this.asset = host.querySelector('[name="setup-asset"]')!;
    this.status = host.querySelector("[data-setup-status]")!;
    host.querySelector("[data-setup-import]")!.addEventListener("click", () => void this.run(true));
    host.querySelector("[data-setup-verify]")!.addEventListener("click", () => void this.run(false));
    host.querySelector("[data-setup-cancel]")!.addEventListener("click", () => this.cancel());
    this.source.addEventListener("input", () => this.controls());
    this.asset.addEventListener("change", () => this.controls());
    this.controls();
  }
  setDisabled(value: boolean): void { this.disabled = value; this.controls(); }
  cancel(): void {
    const generation = ++this.generation, id = this.requestId;
    this.requestId = null;
    if (id) void cancelLocalGeneration(id).catch(error => { if (generation === this.generation) this.status.textContent = `Setup cancellation could not be delivered: ${message(error)}. The native deadline remains active.`; });
    if (this.running) {
      this.running = false; this.busy(false);
      this.status.textContent = "Setup cancelled. An artifact already committed remains installed; native cleanup may still be finishing. Inspect setup before continuing.";
    }
    this.controls();
  }
  private controls(): void {
    const blocked = this.disabled || this.running;
    this.source.disabled = blocked; this.asset.disabled = blocked;
    this.host.querySelector<HTMLButtonElement>("[data-setup-import]")!.disabled = blocked || !this.source.value.startsWith("/");
    this.host.querySelector<HTMLButtonElement>("[data-setup-verify]")!.disabled = blocked;
    this.host.querySelector<HTMLButtonElement>("[data-setup-cancel]")!.disabled = !this.running;
    this.host.querySelector("[data-setup-source-help]")!.textContent = this.asset.value === "model"
      ? "Use Qwen3.5-9B-UD-Q4_K_XL.gguf from unsloth/Qwen3.5-9B-GGUF on Hugging Face (Apache-2.0). The selected file is about 5.97 GB."
      : `Use the extracted ${this.asset.value === "cpu" ? "llama-b12110-bin-ubuntu-x64" : "llama-b12110-bin-ubuntu-vulkan-x64"} package from ggml-org/llama.cpp release b12110 (MIT). Select the folder containing llama-server, libraries and LICENSE, not the archive.`;
    this.host.setAttribute("aria-busy", String(this.running));
  }
  private async run(importing: boolean): Promise<void> {
    if (this.running || this.disabled) return;
    const generation = ++this.generation, asset = this.asset.value as InferenceAsset, source = this.source.value;
    this.running = true; this.busy(true); this.controls();
    this.status.textContent = importing ? "Copying and checking the complete artifact. Keep this view open; large model imports may take time…" : "Checking all installed bytes and package links…";
    let id: string | null = null;
    try {
      id = await reserveLocalGeneration();
      if (generation !== this.generation) { await cancelLocalGeneration(id); return; }
      this.requestId = id;
      const result = importing ? await importInferenceAsset(id, asset, source) : await verifyInferenceAsset(id, asset);
      if (generation !== this.generation) return;
      this.status.textContent = `${result.outcome === "installed" ? "Installed" : result.outcome === "already-installed" ? "Already installed and verified" : "Verified"}: ${result.asset}. ${result.bytes.toLocaleString()} bytes checked. ${result.location}. Artifact identity passed; device compatibility is checked when you run a request.`;
      try {
        const availability = await localInferenceAvailability();
        if (generation === this.generation) this.changed(availability);
      } catch (error) { if (generation === this.generation) this.status.textContent += ` Setup status could not refresh: ${message(error)}`; }
    } catch (error) { if (generation === this.generation) this.status.textContent = `Setup unavailable: ${message(error)}`; }
    finally {
      if (id) await cancelLocalGeneration(id).catch(() => {});
      if (generation === this.generation) { this.requestId = null; this.running = false; this.busy(false); this.controls(); }
    }
  }
}
function message(error: unknown): string { return typeof error === "object" && error !== null && "message" in error ? String(error.message) : String(error); }
