// @vitest-environment jsdom
import { beforeEach, expect, it, vi } from "vitest";
import { LocalInferenceSetup } from "./inference-setup";
const { reserve, cancel, install, verify, availability } = vi.hoisted(() => ({ reserve: vi.fn(), cancel: vi.fn(), install: vi.fn(), verify: vi.fn(), availability: vi.fn() }));
vi.mock("./api", () => ({ reserveLocalGeneration: reserve, cancelLocalGeneration: cancel, importInferenceAsset: install, verifyInferenceAsset: verify, localInferenceAvailability: availability }));
function setup() {
  const host = document.createElement("div"); document.body.replaceChildren(host);
  const changed = vi.fn(), busy = vi.fn(); const control = new LocalInferenceSetup(host, changed, busy);
  const source = host.querySelector<HTMLInputElement>('[name="setup-source"]')!;
  const asset = host.querySelector<HTMLSelectElement>('[name="setup-asset"]')!;
  const start = host.querySelector<HTMLButtonElement>("[data-setup-import]")!;
  const check = host.querySelector<HTMLButtonElement>("[data-setup-verify]")!;
  const choose = (value: string) => { source.value = value; source.dispatchEvent(new Event("input")); };
  return { host, changed, busy, control, source, asset, start, check, choose };
}
const result = { asset: "model", outcome: "installed", bytes: 5966095584, location: "/local/models/Qwen.gguf" };
beforeEach(() => {
  vi.resetAllMocks(); reserve.mockResolvedValue("setup-owner"); cancel.mockResolvedValue(undefined);
  install.mockResolvedValue(result); verify.mockResolvedValue({ ...result, outcome: "verified" });
  availability.mockResolvedValue({ supported: true, model_present: true, vulkan_present: true, cpu_present: false, location: "/local", model: "Qwen9B" });
});
it("imports only after explicit action and refreshes parent availability after checked success", async () => {
  const { host, changed, busy, choose, start, source, check } = setup();
  expect(start.disabled).toBe(true); expect(reserve).not.toHaveBeenCalled(); choose("/downloads/model.gguf"); start.click();
  expect(source.disabled).toBe(true); expect(check.disabled).toBe(true);
  await vi.waitFor(() => expect(changed).toHaveBeenCalled());
  expect(install).toHaveBeenCalledWith("setup-owner", "model", "/downloads/model.gguf");
  expect(host.textContent).toContain("Installed: model"); expect(host.textContent).toContain("device compatibility");
  await vi.waitFor(() => expect(busy).toHaveBeenLastCalledWith(false)); expect(cancel).toHaveBeenCalledWith("setup-owner");
});
it("verifies a selected installed runtime without a source or import", async () => {
  const { host, asset, check } = setup(); asset.value = "cpu"; asset.dispatchEvent(new Event("change"));
  verify.mockResolvedValue({ ...result, asset: "cpu", outcome: "verified" }); check.click();
  await vi.waitFor(() => expect(host.textContent).toContain("Verified: cpu"));
  expect(verify).toHaveBeenCalledWith("setup-owner", "cpu"); expect(install).not.toHaveBeenCalled();
});
it("cancels an obsolete reservation before it can import and suppresses late success", async () => {
  const { host, control, changed, choose, start } = setup();
  let finish!: (id: string) => void; reserve.mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  choose("/local/model.gguf"); start.click(); control.cancel(); finish("obsolete");
  await vi.waitFor(() => expect(cancel).toHaveBeenCalledWith("obsolete")); expect(install).not.toHaveBeenCalled(); expect(changed).not.toHaveBeenCalled();
  reserve.mockResolvedValue("current"); let installed!: (value: unknown) => void; install.mockImplementation(() => new Promise(resolve => { installed = resolve; }));
  start.click(); await vi.waitFor(() => expect(install).toHaveBeenCalled()); control.cancel(); installed(result);
  await vi.waitFor(() => expect(cancel).toHaveBeenCalledWith("current")); expect(changed).not.toHaveBeenCalled(); expect(host.textContent).not.toContain("Installed: model");
});
it("reports refusal and preserves a committed outcome when presence refresh fails", async () => {
  const { host, choose, start, changed } = setup(); choose("/local/changed.gguf"); install.mockRejectedValueOnce({ message: "Existing asset is invalid; it was preserved" }); start.click();
  await vi.waitFor(() => expect(host.textContent).toContain("it was preserved")); expect(changed).not.toHaveBeenCalled();
  await vi.waitFor(() => expect(start.disabled).toBe(false)); availability.mockRejectedValue(new Error("refresh failed")); start.click();
  await vi.waitFor(() => expect(host.textContent).toContain("Setup status could not refresh")); expect(host.textContent).toContain("Installed: model");
});
