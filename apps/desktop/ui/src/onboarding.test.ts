// @vitest-environment jsdom
import { beforeEach, expect, it, vi } from "vitest";
import { OnboardingPanel } from "./onboarding";
import type { OnboardingResult, OnboardingReview } from "./onboarding-types";
const { review, publish, prepare, availability, reserve, cancel, generate, install, verify } = vi.hoisted(() => ({ review: vi.fn(), publish: vi.fn(), prepare: vi.fn(), availability: vi.fn(), reserve: vi.fn(), cancel: vi.fn(), generate: vi.fn(), install: vi.fn(), verify: vi.fn() }));
vi.mock("./api", () => ({ reviewOnboarding: review, publishOnboarding: publish, prepareOnboardingSources: prepare, localInferenceAvailability: availability, reserveLocalGeneration: reserve, cancelLocalGeneration: cancel, generateOnboarding: generate, importInferenceAsset: install, verifyInferenceAsset: verify }));
const binding = { root: "/resolved/root", project: "alpha" };
const id = "sha256:" + "a".repeat(64);
const plan = { schema_version: 5, notes: [{ slot: 1, evidence: [{ kind: "fact", claim: "The limit is 37.", sources: [{ source: 1, excerpt: 1 }] }] }] };
function response(): OnboardingReview {
  return { draft: {
    proposal: { notes: [{ note_type: "entity", path: "limits.md", source: "---\nstatus: draft\nreviewed: unreviewed\n---\nThe limit is 37.\n" }], index: "# Index\n[[limits]]", roadmap: "# Roadmap\n[[limits]]" },
    preview: { ...binding, preview_id: id, notes: [{ path: "entities/limits.md", action: "create", evidence_claims: 1 }], index_changed: true, roadmap_changed: true, state_changed: true },
    review: { requires_review: true, notes: [{ path: "limits.md", wikilink: "[[Projects/alpha/entities/limits.md]]", fields: { status: "draft", reviewed: "unreviewed" } }], uncited_sources: ["tests/missing.rs"], sampled_sources: [{ total_bytes: 35000, omitted_lines: [{ line_start: 17, line_end: 100 }] }] },
  }, identities: { slots: [{ slot: 1, role: "implementation", sources: [1] }] }, excerpts: [
    { source: 1, excerpt: 1, path: "src/limits.rs", line_start: 1, line_end: 2, text: "🌘 <script>unsafe()</script>\r\nconst LIMIT = 37;\r\n" },
    { source: 1, excerpt: 2, path: "src/limits.rs", line_start: 101, line_end: 101, text: "// omitted behavior\n" },
    { source: 2, excerpt: 1, path: "tests/outside.rs", line_start: 1, line_end: 1, text: "// another slot\n" },
  ] };
}
function setup() {
  const host = document.createElement("div"); document.body.replaceChildren(host);
  const published = vi.fn(async (_result: OnboardingResult) => {});
  const panel = new OnboardingPanel(host, published); panel.bind(binding);
  const files = (packet = "{}", claimPlan = JSON.stringify(plan)) => {
    for (const [name, content] of [["packet", packet], ["plan", claimPlan]] as const) {
      const input = host.querySelector<HTMLInputElement>(`[name="${name}"]`)!;
      Object.defineProperty(input, "files", { configurable: true, value: [{ size: new TextEncoder().encode(content).length, text: async () => content }] });
      input.dispatchEvent(new Event("change"));
    }
  };
  const load = () => host.querySelector<HTMLButtonElement>("[data-preview]")!.click();
  const approval = host.querySelector<HTMLInputElement>('[name="approval"]')!;
  const approve = (value = id) => { approval.value = value; approval.dispatchEvent(new Event("input")); };
  const apply = host.querySelector<HTMLButtonElement>("[data-publish]")!;
  const ready = async () => { files(); load(); await vi.waitFor(() => expect(host.textContent).toContain("Fresh preview ready")); };
  return { host, panel, published, files, load, approval, approve, apply, ready };
}
beforeEach(() => { review.mockReset(); publish.mockReset(); prepare.mockReset(); review.mockResolvedValue(response()); availability.mockReset(); reserve.mockReset(); cancel.mockReset(); generate.mockReset(); cancel.mockResolvedValue(undefined); });
it("renders literal source, metadata and gaps and requires full-ID explicit publication", async () => {
  const { host, ready, approve, apply, published } = setup();
  expect(review).not.toHaveBeenCalled(); expect(publish).not.toHaveBeenCalled(); await ready();
  expect(review).toHaveBeenCalledWith(binding.root, binding.project, "{}", JSON.stringify(plan));
  expect(host.textContent).toContain("🌘 <script>unsafe()</script>"); expect(host.querySelector("script")).toBeNull();
  expect(host.textContent).not.toContain("tests/outside.rs"); expect(host.textContent).toContain("unreviewed");
  expect(host.textContent).toContain("tests/missing.rs"); expect(host.textContent).toContain("35000"); expect(host.textContent).toContain("Exact proposed index");
  approve("a".repeat(64)); apply.click(); expect(publish).not.toHaveBeenCalled(); approve();
  publish.mockResolvedValue({ ...binding, created_notes: ["limits.md"], unchanged_notes: [], updated_projections: [], state: "state" });
  apply.click(); await vi.waitFor(() => expect(published).toHaveBeenCalledOnce());
  expect(publish).toHaveBeenCalledWith(binding.root, binding.project, "{}", JSON.stringify(plan), id);
  expect(apply.disabled).toBe(true); apply.click(); expect(publish).toHaveBeenCalledOnce();
});
it("invalidates approval after corrections and preserves slot metadata and source groups", async () => {
  const { host, ready, approve, apply, load } = setup(); await ready(); approve();
  const wording = host.querySelector<HTMLTextAreaElement>('[aria-label="Note 1 claim 1"]')!;
  wording.value = "The capacity is exactly 37."; wording.dispatchEvent(new Event("input")); expect(apply.disabled).toBe(true);
  const add = [...host.querySelectorAll<HTMLButtonElement>("button")].filter(button => button.textContent === "Add claim citing this excerpt"); add[1]!.click();
  const second = host.querySelector<HTMLTextAreaElement>('[aria-label="Note 1 claim 2"]')!;
  second.value = "A second reviewed behavior."; second.dispatchEvent(new Event("input"));
  load(); await vi.waitFor(() => expect(review).toHaveBeenCalledTimes(2));
  expect(JSON.parse(review.mock.calls[1]![3])).toEqual({ schema_version: 5, notes: [{ slot: 1, evidence: [
    { ...plan.notes[0]!.evidence[0], claim: wording.value }, { kind: "fact", claim: second.value, sources: [{ source: 1, excerpt: 2 }] },
  ] }] });
  await vi.waitFor(() => expect(host.textContent).toContain("Fresh preview ready")); expect(apply.disabled).toBe(true);
});
it("enforces claim bounds and clears citations when a claim becomes unknown", async () => {
  const { host, ready, load } = setup(); await ready();
  const add = () => [...host.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "Add claim citing this excerpt")!;
  const remove = () => [...host.querySelectorAll<HTMLButtonElement>("button")].filter(button => button.textContent === "Remove claim");
  expect(remove()[0]!.disabled).toBe(true); add().click(); add().click(); add().click(); expect(add().disabled).toBe(true);
  expect(host.querySelectorAll(".onboarding-claim")).toHaveLength(4); remove()[3]!.click(); expect(host.querySelectorAll(".onboarding-claim")).toHaveLength(3);
  const kind = host.querySelector<HTMLSelectElement>('[aria-label="Evidence kind"]')!; kind.value = "unknown"; kind.dispatchEvent(new Event("change"));
  load(); await vi.waitFor(() => expect(review).toHaveBeenCalledTimes(2)); expect(JSON.parse(review.mock.calls[1]![3]).notes[0].evidence[0].sources).toEqual([]);
});
it("suppresses late previews after replacement, close, dirty edits and project changes", async () => {
  const pending: Array<(value: OnboardingReview) => void> = [];
  review.mockImplementation(() => new Promise<OnboardingReview>(resolve => pending.push(resolve)));
  const { host, panel, files, load } = setup();
  const start = async () => { files(); load(); await vi.waitFor(() => expect(pending.length).toBe(1)); };
  await start(); files(); pending.shift()!(response()); await Promise.resolve(); expect(host.querySelector("article")).toBeNull();
  await start(); panel.suspend(); pending.shift()!(response()); await Promise.resolve(); expect(host.querySelector("article")).toBeNull();
  await start(); panel.setDirty(true); panel.setDirty(false); pending.shift()!(response()); await Promise.resolve(); expect(host.querySelector("article")).toBeNull();
  await start(); panel.bind({ ...binding, project: "beta" }); pending.shift()!(response()); await Promise.resolve(); expect(host.textContent).toContain("beta"); expect(host.querySelector("article")).toBeNull();
});
it("keeps publication bound to its project and consumes approval after refusal", async () => {
  const { host, panel, ready, approve, apply, published } = setup(); await ready(); approve();
  let reject!: (error: unknown) => void;
  publish.mockImplementation(() => new Promise<OnboardingResult>((_, fail) => { reject = fail; })); apply.click();
  expect(panel.isPublishing).toBe(true); expect(panel.suspend()).toBe(false); panel.bind({ ...binding, project: "beta" });
  expect(host.querySelector<HTMLInputElement>('[name="packet"]')!.disabled).toBe(true);
  reject({ code: 5, message: "source changed" }); await vi.waitFor(() => expect(host.textContent).toContain("source changed"));
  expect(panel.isPublishing).toBe(false); expect(apply.disabled).toBe(true); expect(published).not.toHaveBeenCalled(); approve(); apply.click(); expect(publish).toHaveBeenCalledOnce();
});
it("refuses input overflow and foreign results and separates commit from refresh failures", async () => {
  const view = setup(); view.load(); await vi.waitFor(() => expect(view.host.textContent).toContain("Choose a JSON file"));
  view.files("x".repeat(16001)); view.load(); await vi.waitFor(() => expect(view.host.textContent).toContain("16000 bytes")); expect(review).not.toHaveBeenCalled();
  const foreign = response(); foreign.draft.preview.root = "/foreign/root"; review.mockResolvedValueOnce(foreign);
  view.files(); view.load(); await vi.waitFor(() => expect(view.host.textContent).toContain("another project or root")); expect(view.host.querySelector("article")).toBeNull();
  await view.ready(); view.approve(); publish.mockResolvedValue({ ...binding, created_notes: ["limits.md"], unchanged_notes: [], updated_projections: [], state: "state" });
  view.published.mockRejectedValueOnce(new Error("refresh unavailable")); view.apply.click();
  await vi.waitFor(() => expect(view.host.textContent).toContain("Publication succeeded. Library refresh failed")); expect(view.apply.disabled).toBe(true);
});

it("binds metadata and paths to slot identities when the imported notes are reordered", async () => {
  const result = response();
  result.identities!.slots.push({ slot: 2, role: "verification", sources: [2] });
  result.draft.review.notes.push({ path: "verification.md", wikilink: "[[Projects/alpha/entities/verification.md]]", fields: { entity: "verification" } });
  review.mockResolvedValueOnce(result);
  const { host, files, load } = setup();
  files("{}", JSON.stringify({ schema_version: 5, notes: [
    { slot: 2, evidence: [{ kind: "fact", claim: "A test assertion.", sources: [{ source: 2, excerpt: 1 }] }] }, plan.notes[0],
  ] }));
  load(); await vi.waitFor(() => expect(host.textContent).toContain("Fresh preview ready"));
  const notes = host.querySelectorAll(".onboarding-note");
  expect(notes[0]!.querySelector("h3")!.textContent).toBe("verification.md");
  expect(notes[0]!.textContent).toContain("tests/outside.rs");
  expect(notes[0]!.textContent).not.toContain("src/limits.rs");
  expect(notes[1]!.querySelector("h3")!.textContent).toBe("limits.md");
  expect(notes[1]!.textContent).not.toContain("tests/outside.rs");
  // Targets come from the core review order, even when the editable plan is reordered.
  const wording = notes[0]!.querySelector<HTMLTextAreaElement>('textarea[aria-label="Note 1 claim 1"]')!;
  const insert = [...notes[0]!.querySelectorAll("button")].find(button => button.textContent === "Insert related note link")!;
  insert.click();
  expect(wording.value).toBe("A test assertion. Related: [[Projects/alpha/entities/limits.md]].");
  expect(host.querySelector<HTMLInputElement>('[name="approval"]')!.disabled).toBe(true);
  expect(insert.disabled).toBe(true);
  const undo = [...notes[0]!.querySelectorAll("button")].find(button => button.textContent === "Undo link insertion")!;
  undo.click(); expect(wording.value).toBe("A test assertion."); expect(insert.disabled).toBe(false);
  insert.click(); wording.value = "Manual correction."; wording.dispatchEvent(new Event("input"));
  expect(undo.disabled).toBe(true);
  load(); await vi.waitFor(() => expect(review).toHaveBeenCalledTimes(2));
  const submitted = JSON.parse(review.mock.calls[1]![3]);
  expect(submitted.notes[0].evidence[0].claim).toBe("Manual correction.");
  expect(submitted.notes[0].evidence[0].sources).toEqual([{ source: 2, excerpt: 1 }]);
});

function prepared() {
  return { ...binding, packet_json: '{"captured":"exact 世界"}', identities: { slots: [{ slot: 1, role: "implementation", path: "limits.md", sources: [1], fields: { status: "draft" }, unresolved_fields: [] }] }, excerpts: response().excerpts.slice(0, 2), sampled_sources: [{ omitted_lines: [{ line_start: 3, line_end: 100 }] }], coverage: { directories: ["src"], entries_inspected: 12, candidates: 3, selected: [{ path: "src/limits.rs", role: "implementation" }], implementation_groups: [{ directory: "src/missing", candidates: 2, selected: 0 }], omitted: [{ path: "src/missing/other.rs", reason: "source count budget" }], excluded_entries: 1, excluded_examples: [{ path: ".secret", reason: "excluded" }] } };
}
function sourceControls(host: HTMLElement) {
  const paths = host.querySelector<HTMLTextAreaElement>('[name="source-paths"]')!;
  const inspect = host.querySelector<HTMLButtonElement>("[data-prepare-sources]")!;
  const set = (value: string) => { paths.value = value; paths.dispatchEvent(new Event("input")); };
  return { paths, inspect, set };
}
it("starts from native sources, exposes omissions and retains the exact packet for plan-only import", async () => {
  prepare.mockResolvedValue(prepared());
  const { host, load, approve, apply } = setup(); const { set, inspect } = sourceControls(host);
  expect(prepare).not.toHaveBeenCalled(); set("src\n tests\n");
  const focus = host.querySelector<HTMLTextAreaElement>('[name="source-focus"]')!; focus.value = "lease\n deadline ";
  host.querySelector<HTMLInputElement>('[name="source-sampling"]')!.checked = true; inspect.click();
  await vi.waitFor(() => expect(host.textContent).toContain("Sources ready"));
  expect(prepare).toHaveBeenCalledWith(binding.root, binding.project, { mode: "discover", paths: ["src", "tests"], focus: ["lease", "deadline"], sample_large_files: true });
  expect(host.textContent).toContain("Missing implementation group: src/missing");
  expect(host.textContent).toContain("source count budget"); expect(host.textContent).toContain("omitted_lines");
  expect(host.querySelector("script")).toBeNull();
  expect(host.querySelector<HTMLTextAreaElement>('[aria-label="Exact source packet"]')!.value).toBe(prepared().packet_json);
  const input = host.querySelector<HTMLInputElement>('[name="plan"]')!;
  Object.defineProperty(input, "files", { value: [{ size: 500, text: async () => JSON.stringify(plan) }] }); input.dispatchEvent(new Event("change"));
  load(); await vi.waitFor(() => expect(host.textContent).toContain("Fresh preview ready"));
  expect(review).toHaveBeenCalledWith(binding.root, binding.project, prepared().packet_json, JSON.stringify(plan));
  approve(); expect(apply.disabled).toBe(false); set("src/other.rs"); expect(apply.disabled).toBe(true);
  expect(host.querySelector(".onboarding-claim")).toBeNull(); expect(host.querySelector('[aria-label="Exact source packet"]')).toBeNull();
});
it("replaces directory coverage with an editable explicit file selection and clears previous approval", async () => {
  prepare.mockResolvedValue(prepared()); const { host } = setup(); const { set, inspect, paths } = sourceControls(host);
  set("src"); inspect.click(); await vi.waitFor(() => expect(host.textContent).toContain("Sources ready"));
  [...host.querySelectorAll<HTMLButtonElement>("button")].find(value => value.textContent === "Use selected paths as an editable file list")!.click();
  expect(paths.value).toBe("src/limits.rs"); expect(host.querySelector<HTMLSelectElement>('[name="source-mode"]')!.value).toBe("files");
  expect(host.textContent).not.toContain("Missing implementation group");
  prepare.mockResolvedValue({ ...prepared(), coverage: null }); inspect.click();
  await vi.waitFor(() => expect(prepare).toHaveBeenCalledTimes(2));
  expect(prepare.mock.calls[1]![2].mode).toBe("files");
  await vi.waitFor(() => expect(host.textContent).toContain("Explicit file selection. No directory inventory"));
});
it("suppresses late native inspections after cancellation, close, input replacement and project switch", async () => {
  for (const action of ["cancel", "close", "packet", "plan", "project"]) {
    let finish!: (value: ReturnType<typeof prepared>) => void;
    prepare.mockImplementation(() => new Promise(resolve => { finish = resolve; }));
    const { host, panel, files } = setup(); const { set, inspect } = sourceControls(host); set("src"); inspect.click();
    if (action === "cancel") host.querySelector<HTMLButtonElement>("[data-cancel-sources]")!.click();
    if (action === "close") panel.suspend();
    if (action === "packet") files();
    if (action === "plan") host.querySelector<HTMLInputElement>('[name="plan"]')!.dispatchEvent(new Event("change"));
    if (action === "project") panel.bind({ ...binding, project: "beta" });
    finish(prepared()); await Promise.resolve(); await Promise.resolve();
    expect(host.querySelector('[aria-label="Exact source packet"]')).toBeNull();
  }
});
it("refuses foreign preparation results and locks source selection during publication", async () => {
  prepare.mockResolvedValue({ ...prepared(), root: "/other" });
  const { host, ready, approve, apply } = setup(); const { set, inspect, paths } = sourceControls(host); set("src"); inspect.click();
  await vi.waitFor(() => expect(host.textContent).toContain("Sources belong to another project"));
  expect(host.querySelector('[aria-label="Exact source packet"]')).toBeNull();
  await ready(); approve(); let finish!: (value: OnboardingResult) => void;
  publish.mockImplementation(() => new Promise(resolve => { finish = resolve; })); apply.click();
  expect(paths.disabled).toBe(true); expect(inspect.disabled).toBe(true);
  finish({ ...binding, created_notes: [], unchanged_notes: [], updated_projections: [], state: "state" });
  await vi.waitFor(() => expect(paths.disabled).toBe(false));
});

async function nativeReady(host: HTMLElement) {
  prepare.mockResolvedValue(prepared());
  const controls = sourceControls(host); controls.set("src"); controls.inspect.click();
  await vi.waitFor(() => expect(host.textContent).toContain("Sources ready"));
  availability.mockResolvedValue({ supported: true, model_present: true, vulkan_present: true, cpu_present: false, location: "/local/inference", model: "Qwen9B" });
  host.querySelector<HTMLButtonElement>("[data-inference-setup]")!.click();
  await vi.waitFor(() => expect(host.textContent).toContain("Exact SHA-256 checks"));
  return host.querySelector<HTMLButtonElement>("[data-generate]")!;
}
it("generates from exact selected packet and still requires fresh preview and explicit approval", async () => {
  const { host, load, approve, apply } = setup(); const start = await nativeReady(host);
  reserve.mockResolvedValue("native-1"); generate.mockResolvedValue({ plan_json: JSON.stringify(plan), review: response(), metrics: [{}] });
  start.click(); await vi.waitFor(() => expect(host.textContent).toContain("Worker stopped"));
  expect(generate).toHaveBeenCalledWith("native-1", binding.root, binding.project, prepared().packet_json, "vulkan");
  expect(host.querySelector('[aria-label="Note 1 claim 1"]')).not.toBeNull();
  approve(); expect(apply.disabled).toBe(true); expect(publish).not.toHaveBeenCalled();
  load(); await vi.waitFor(() => expect(host.textContent).toContain("Fresh preview ready")); approve(); expect(apply.disabled).toBe(false);
});
it("cancels an active native worker on close and discards a late generated draft", async () => {
  const { host, panel } = setup(); const start = await nativeReady(host); reserve.mockResolvedValue("native-close");
  let finish!: (value: unknown) => void; generate.mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  start.click(); await vi.waitFor(() => expect(generate).toHaveBeenCalled());
  expect(sourceControls(host).inspect.disabled).toBe(true); panel.suspend();
  expect(cancel).toHaveBeenCalledWith("native-close"); finish({ plan_json: JSON.stringify(plan), review: response(), metrics: [{}] });
  await vi.waitFor(() => expect(host.querySelector('[data-native-generation]')!.getAttribute("aria-busy")).toBe("false"));
  expect(host.querySelector('[aria-label="Note 1 claim 1"]')).toBeNull(); expect(publish).not.toHaveBeenCalled();
});
it("cancels a reservation that returns after a project switch, before starting a worker", async () => {
  const { host, panel } = setup(); const start = await nativeReady(host);
  let reserved!: (id: string) => void; reserve.mockImplementation(() => new Promise(resolve => { reserved = resolve; }));
  start.click(); panel.bind({ ...binding, project: "beta" }); reserved("obsolete-reservation");
  await vi.waitFor(() => expect(cancel).toHaveBeenCalledWith("obsolete-reservation")); expect(generate).not.toHaveBeenCalled();
});
it("keeps generation unavailable for missing artifacts and reports native refusals without publication", async () => {
  const { host } = setup(); const start = await nativeReady(host);
  host.querySelector<HTMLSelectElement>('[name="inference-profile"]')!.value = "cpu";
  host.querySelector('[name="inference-profile"]')!.dispatchEvent(new Event("change")); expect(start.disabled).toBe(true);
  host.querySelector<HTMLSelectElement>('[name="inference-profile"]')!.value = "vulkan";
  host.querySelector('[name="inference-profile"]')!.dispatchEvent(new Event("change"));
  reserve.mockResolvedValue("refused"); generate.mockRejectedValue({message:"Model SHA-256 mismatch"}); start.click();
  await vi.waitFor(() => expect(host.textContent).toContain("Generation unavailable: Model SHA-256 mismatch"));
  expect(cancel).toHaveBeenCalledWith("refused"); expect(publish).not.toHaveBeenCalled();
});
it("installs through shared setup, refreshes generation availability and cancels setup on project change", async () => {
  const { host, panel } = setup(); prepare.mockResolvedValue(prepared());
  const controls = sourceControls(host); controls.set("src"); controls.inspect.click();
  await vi.waitFor(() => expect(host.textContent).toContain("Sources ready"));
  reserve.mockResolvedValue("setup-import"); install.mockResolvedValue({ asset: "model", outcome: "installed", bytes: 5966095584, location: "/local/model" });
  availability.mockResolvedValue({ supported: true, model_present: true, vulkan_present: true, cpu_present: false, location: "/local", model: "Qwen9B" });
  const source = host.querySelector<HTMLInputElement>('[name="setup-source"]')!;
  source.value = "/downloads/Qwen.gguf"; source.dispatchEvent(new Event("input"));
  const start = host.querySelector<HTMLButtonElement>("[data-setup-import]")!; start.click();
  expect(host.querySelector<HTMLButtonElement>("[data-generate]")!.disabled).toBe(true);
  await vi.waitFor(() => expect(host.querySelector<HTMLButtonElement>("[data-generate]")!.disabled).toBe(false));
  expect(install).toHaveBeenCalledWith("setup-import", "model", source.value); expect(generate).not.toHaveBeenCalled(); expect(publish).not.toHaveBeenCalled();
  let finish!: (value: unknown) => void; install.mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
  start.click(); await vi.waitFor(() => expect(start.disabled).toBe(true));
  await vi.waitFor(() => expect(install).toHaveBeenCalledTimes(2)); panel.bind({ ...binding, project: "beta" });
  expect(cancel).toHaveBeenCalledWith("setup-import"); finish({ asset: "model", outcome: "installed", bytes: 1, location: "/obsolete" });
  await vi.waitFor(() => expect(start.disabled).toBe(false)); expect(host.textContent).not.toContain("/obsolete");
});
