// @vitest-environment jsdom
import { beforeEach, expect, it, vi } from "vitest";
import { OnboardingPanel } from "./onboarding";
import type { OnboardingResult, OnboardingReview } from "./onboarding-types";
const { review, publish } = vi.hoisted(() => ({ review: vi.fn(), publish: vi.fn() }));
vi.mock("./api", () => ({ reviewOnboarding: review, publishOnboarding: publish }));
const binding = { root: "/resolved/root", project: "alpha" };
const id = "sha256:" + "a".repeat(64);
const plan = { schema_version: 5, notes: [{ slot: 1, evidence: [{ kind: "fact", claim: "The limit is 37.", sources: [{ source: 1, excerpt: 1 }] }] }] };
function response(): OnboardingReview {
  return { draft: {
    proposal: { notes: [{ note_type: "entity", path: "limits.md", source: "---\nstatus: draft\nreviewed: unreviewed\n---\nThe limit is 37.\n" }], index: "# Index\n[[limits]]", roadmap: "# Roadmap\n[[limits]]" },
    preview: { ...binding, preview_id: id, notes: [{ path: "entities/limits.md", action: "create", evidence_claims: 1 }], index_changed: true, roadmap_changed: true, state_changed: true },
    review: { requires_review: true, notes: [{ path: "limits.md", fields: { status: "draft", reviewed: "unreviewed" } }], uncited_sources: ["tests/missing.rs"], sampled_sources: [{ total_bytes: 35000, omitted_lines: [{ line_start: 17, line_end: 100 }] }] },
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
beforeEach(() => { review.mockReset(); publish.mockReset(); review.mockResolvedValue(response()); });
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
  const kind = host.querySelector<HTMLSelectElement>("select")!; kind.value = "unknown"; kind.dispatchEvent(new Event("change"));
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
  result.draft.review.notes.push({ path: "verification.md", fields: { entity: "verification" } });
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
});
