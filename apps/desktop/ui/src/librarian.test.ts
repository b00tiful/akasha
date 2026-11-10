// @vitest-environment jsdom
import { beforeEach, expect, it, vi } from "vitest";
import { LibrarianPanel } from "./librarian";
import type { DesktopLibrarianReview } from "./librarian-types";
import type { LinkedLibraryDocument } from "./types";

const { inspect, follow, setupModel, reserve, cancel, assist } = vi.hoisted(() => ({ inspect: vi.fn(), follow: vi.fn(), setupModel: vi.fn(), reserve: vi.fn(), cancel: vi.fn(), assist: vi.fn() }));
vi.mock("./api", () => ({ inspectLibrarian: inspect, followLibrarianSource: follow, localInferenceAvailability: setupModel, reserveLocalGeneration: reserve, cancelLocalGeneration: cancel, assistLibrarian: assist }));
const binding = { root: "/resolved/root", project: "alpha" };
const id = "Projects/alpha/entities/core.md";
function result(): DesktopLibrarianReview {
  return { packet: {
    schema_version: 1, project: "alpha", question: "core", terms: [], note: null,
    scanned_notes: 18, matched_notes: 9, omitted_notes: 3,
    evidence: [{ citation: 1, id, fingerprint: "sha256:hash", status: "draft", line_start: 8, line_end: 9,
      excerpt: "🌘 <script>hostile()</script>\r\nThe limit is 37.\r\n", excerpt_truncated: false }],
  }, review: {
    schema_version: 1, kind: "source_review", highlighted_citations: [],
    scanned_notes: 18, matched_notes: 9, omitted_notes: 3,
    sources: { schema_version: 1, kind: "source_passages", project: "alpha", question: "core", link_scope: "packet",
      matches: [{ citation: 1, id, fingerprint: "sha256:hash", status: "draft", line_start: 8, line_end: 9,
        partial_body: true, quote: "🌘 <script>hostile()</script>\r\nThe limit is 37.\r\n", links: [], omitted_links: 2 }] },
    provenance: { association: "exact", notes: [{ citation: 1, evidence_state: "recorded", unmatched_claims: 1,
      claims: [{ recorded_kind: "fact", claim: "The limit is 37.", rationale: null, note_line: 9,
        sources: [{ path: "src/limits.rs", recorded_fingerprint: "sha256:hash", observed_fingerprint: "sha256:hash",
          line_start: 2, line_end: 2, freshness: "unchanged", excerpt_state: "complete", excerpt: "const LIMIT: usize = 37;\r\n" }] }] }] },
  } };
}
function panel() {
  const host = document.createElement("div");
  document.body.replaceChildren(host);
  const activate = vi.fn();
  const panel = new LibrarianPanel(host, activate);
  panel.bind(binding);
  const question = host.querySelector<HTMLInputElement>('[name="question"]')!;
  question.value = "core";
  const submit = () => host.querySelector("form")!.dispatchEvent(new Event("submit", { cancelable: true }));
  const open = () => host.querySelector<HTMLButtonElement>("article > button")!.click();
  return { host, panel, question, submit, open, activate };
}
beforeEach(() => {
  for (const mock of [inspect, follow, setupModel, reserve, cancel, assist]) mock.mockReset();
  cancel.mockResolvedValue(undefined);
  reserve.mockResolvedValue("reserved-local-id");
  setupModel.mockResolvedValue({ supported: true, model_present: true, vulkan_present: true, cpu_present: false, model: "Qwen9B", location: "/installed" });
});

it("shows all checked passages, physical ranges, status, omissions and code without executing source markup", async () => {
  inspect.mockResolvedValue(result());
  const { host, submit, open, activate } = panel();
  expect(inspect).not.toHaveBeenCalled();
  host.querySelector<HTMLInputElement>('[name="provenance"]')!.checked = true;
  submit();
  await vi.waitFor(() => expect(host.querySelector("article")).not.toBeNull());
  expect(inspect).toHaveBeenCalledWith(binding.root, binding.project, "core", true);
  expect(host.textContent).toContain("1 passages / 9 matched / 18 scanned / 3 omitted");
  expect(host.textContent).toContain("Lines 8–9 / recorded status: draft / partial note body");
  expect(host.textContent).toContain("2 recorded links omitted");
  expect(host.textContent).toContain("fact / note line 9");
  expect(host.textContent).toContain("1 recorded claims outside");
  expect(host.querySelector("script")).toBeNull();
  expect(host.querySelectorAll("pre")[0]!.textContent).toBe(result().review.sources.matches[0]!.quote);
  expect(host.querySelectorAll("pre")[1]!.textContent).toBe("const LIMIT: usize = 37;\r\n");
  const document = { id, source: "# Core\n", links: [] };
  follow.mockResolvedValue(document);
  open();
  await vi.waitFor(() => expect(activate).toHaveBeenCalledWith(document));
  expect(follow).toHaveBeenCalledWith(binding.root, binding.project, result().packet, 1);
});

it("renders empty and changed-code results explicitly and requires opt-in for repository reads", async () => {
  const empty = result(); empty.packet.evidence = []; empty.review.sources.matches = [];
  delete empty.review.provenance;
  inspect.mockResolvedValueOnce(empty);
  const { host, submit } = panel();
  submit();
  await vi.waitFor(() => expect(host.textContent).toContain("No saved source matches"));
  expect(inspect).toHaveBeenCalledWith(binding.root, binding.project, "core", false);
  const changed = result();
  changed.review.provenance!.notes[0]!.claims[0]!.sources[0]!.freshness = "changed";
  changed.review.provenance!.notes[0]!.claims[0]!.sources[0]!.excerpt = null;
  inspect.mockResolvedValueOnce(changed);
  submit();
  await vi.waitFor(() => expect(host.textContent).toContain("changed"));
  expect(host.querySelectorAll("pre")).toHaveLength(1);
  expect(host.textContent).not.toContain("const LIMIT");
});

it("retains failed-refresh evidence but disables stale and detached controls", async () => {
  inspect.mockResolvedValueOnce(result());
  const { host, submit, activate } = panel();
  submit();
  await vi.waitFor(() => expect(host.querySelector("article")).not.toBeNull());
  const old = host.querySelector<HTMLButtonElement>("article > button")!;
  inspect.mockRejectedValueOnce({ message: "pending journal" });
  submit();
  await vi.waitFor(() => expect(host.textContent).toContain("Previous results are stale"));
  old.click();
  expect(follow).not.toHaveBeenCalled();
  inspect.mockResolvedValueOnce(result());
  submit();
  await vi.waitFor(() => expect(host.querySelector<HTMLButtonElement>("article > button")!.disabled).toBe(false));
  old.click();
  expect(activate).not.toHaveBeenCalled();
  expect(follow).not.toHaveBeenCalled();
});

it("rejects late query results after cancellation, query edits, closing and project switches", async () => {
  const pending: Array<{ resolve(value: DesktopLibrarianReview): void; reject(error: unknown): void }> = [];
  inspect.mockImplementation(() => new Promise<DesktopLibrarianReview>((resolve, reject) => pending.push({ resolve, reject })));
  const { host, panel: view, question, submit } = panel();
  submit();
  host.querySelector<HTMLButtonElement>("[data-cancel]")!.click();
  pending[0]!.resolve(result());
  await Promise.resolve();
  expect(host.querySelector("article")).toBeNull();
  submit();
  question.value = "different"; question.dispatchEvent(new Event("input"));
  pending[1]!.reject(new Error("obsolete failure"));
  await Promise.resolve();
  expect(host.textContent).not.toContain("obsolete failure");
  submit(); view.suspend(); pending[2]!.resolve(result());
  await Promise.resolve();
  expect(host.querySelector("article")).toBeNull();
  submit(); view.bind({ ...binding, project: "beta" }); pending[3]!.resolve(result());
  await Promise.resolve();
  expect(host.textContent).toContain("beta");
  expect(host.querySelector("article")).toBeNull();
  expect(question.value).toBe("");
});

it("allows inspection of saved files with dirty buffers but prevents both immediate and delayed navigation", async () => {
  inspect.mockResolvedValue(result());
  const { host, panel: view, submit, open, activate } = panel();
  view.setDirty(true); submit();
  await vi.waitFor(() => expect(host.querySelector("article")).not.toBeNull());
  expect(host.textContent).toContain("Unsaved changes");
  open(); expect(follow).not.toHaveBeenCalled();
  view.setDirty(false);
  let resolve!: (document: LinkedLibraryDocument) => void;
  follow.mockImplementation(() => new Promise<LinkedLibraryDocument>(done => { resolve = done; }));
  open(); view.setDirty(true); view.setDirty(false); // A later discard cannot revive an obsolete open.
  resolve({ id, source: "# Core\n", links: [] });
  await Promise.resolve();
  expect(activate).not.toHaveBeenCalled();
});

it("rejects cross-project responses, changed packets on open and identity mismatches", async () => {
  const foreign = result(); foreign.review.sources.project = "beta";
  inspect.mockResolvedValueOnce(foreign);
  const { host, submit, open, activate } = panel();
  submit();
  await vi.waitFor(() => expect(host.textContent).toContain("another project"));
  expect(host.querySelector("article")).toBeNull();
  inspect.mockResolvedValue(result()); submit();
  await vi.waitFor(() => expect(host.querySelector("article")).not.toBeNull());
  follow.mockRejectedValueOnce({ message: "stale source" }); open();
  await vi.waitFor(() => expect(host.textContent).toContain("stale source"));
  expect(activate).not.toHaveBeenCalled();
  submit();
  await vi.waitFor(() => expect(host.querySelector<HTMLButtonElement>("article > button")!.disabled).toBe(false));
  follow.mockResolvedValueOnce({ id: "Global/entities/foreign.md", source: "foreign", links: [] }); open();
  await vi.waitFor(() => expect(host.textContent).toContain("does not match its citation"));
  expect(activate).not.toHaveBeenCalled();
});

async function readyAssistance(host: HTMLElement) {
  expect(host.querySelector<HTMLButtonElement>("[data-assisted]")!.disabled).toBe(true);
  host.querySelector<HTMLButtonElement>("[data-model-setup]")!.click();
  await vi.waitFor(() => expect(host.querySelector<HTMLButtonElement>("[data-assisted]")!.disabled).toBe(false));
}
function assistedResult() {
  const initial = result(), expanded = result();
  expanded.packet.terms = ["capacity", "容量"];
  const extra = { ...expanded.packet.evidence[0]!, citation: 2, id: "Projects/alpha/entities/worker.md", excerpt: "Retry limit 3." };
  expanded.packet.evidence.push(extra);
  expanded.review.sources.matches.push({ ...expanded.review.sources.matches[0]!, ...extra, quote: extra.excerpt });
  return { initial, expanded, terms: expanded.packet.terms, raw_expansion: '{"terms":["capacity","容量"]}', metrics: {} };
}

it("explicitly assists with the installed model, retains initial results and binds additional navigation to its own packet", async () => {
  const response = assistedResult(); assist.mockResolvedValue(response);
  const { host, activate } = panel();
  await readyAssistance(host);
  host.querySelector<HTMLButtonElement>("[data-assisted]")!.click();
  await vi.waitFor(() => expect(host.querySelectorAll("article")).toHaveLength(2));
  expect(assist).toHaveBeenCalledWith("reserved-local-id", binding.root, binding.project, "core", false, "vulkan");
  expect(inspect).not.toHaveBeenCalled();
  expect(host.textContent).toContain("Local search words: capacity, 容量");
  expect(host.textContent).toContain("Original search");
  expect(host.textContent).toContain("Additional local search");
  const document = { id: response.expanded.packet.evidence[1]!.id, source: "Retry limit 3.", links: [] };
  follow.mockResolvedValue(document);
  host.querySelectorAll<HTMLButtonElement>("article > button")[1]!.click();
  await vi.waitFor(() => expect(activate).toHaveBeenCalledWith(document));
  expect(follow).toHaveBeenCalledWith(binding.root, binding.project, response.expanded.packet, 2);
  expect(cancel).toHaveBeenCalledWith("reserved-local-id");
});

it("cancels late reservations without starting a worker and suppresses cancelled completions on project replacement", async () => {
  let reserved!: (id: string) => void;
  reserve.mockImplementationOnce(() => new Promise<string>(done => { reserved = done; }));
  const { host, panel: view } = panel();
  await readyAssistance(host);
  host.querySelector<HTMLButtonElement>("[data-assisted]")!.click();
  host.querySelector<HTMLButtonElement>("[data-cancel]")!.click();
  reserved("late-reservation");
  await vi.waitFor(() => expect(cancel).toHaveBeenCalledWith("late-reservation"));
  expect(assist).not.toHaveBeenCalled();
  let finish!: (value: ReturnType<typeof assistedResult>) => void;
  assist.mockImplementation(() => new Promise(done => { finish = done; }));
  host.querySelector<HTMLButtonElement>("[data-assisted]")!.click();
  await vi.waitFor(() => expect(assist).toHaveBeenCalledTimes(1));
  view.bind({ ...binding, project: "beta" });
  expect(cancel).toHaveBeenCalledWith("reserved-local-id");
  finish(assistedResult()); await Promise.resolve();
  expect(host.querySelector("article")).toBeNull();
  expect(host.textContent).toContain("beta");
});

it("keeps literal searching available when setup or assistance fails and never hides missing support", async () => {
  const { host, question, submit } = panel();
  await readyAssistance(host);
  question.value = "a".repeat(161); question.dispatchEvent(new Event("input"));
  expect(host.querySelector<HTMLButtonElement>("[data-assisted]")!.disabled).toBe(true);
  expect(host.querySelector<HTMLButtonElement>('[type="submit"]')!.disabled).toBe(false);
  question.value = "core"; question.dispatchEvent(new Event("input"));
  assist.mockRejectedValueOnce({ message: "Another local generation is still active" });
  host.querySelector<HTMLButtonElement>("[data-assisted]")!.click();
  await vi.waitFor(() => expect(host.textContent).toContain("still active"));
  const response = assistedResult(); response.expanded = null as unknown as typeof response.expanded; response.terms = [];
  assist.mockResolvedValueOnce(response);
  host.querySelector<HTMLButtonElement>("[data-assisted]")!.click();
  await vi.waitFor(() => expect(host.textContent).toContain("none; literal results retained"));
  expect(host.textContent).toContain("do not establish an answer");
  inspect.mockResolvedValueOnce(result()); submit();
  await vi.waitFor(() => expect(inspect).toHaveBeenCalledTimes(1));
});

it("repairs suggested vocabulary with an explicit model-free search while retaining the question", async () => {
  assist.mockResolvedValueOnce(assistedResult());
  const { host, submit, panel: view } = panel();
  await readyAssistance(host);
  host.querySelector<HTMLButtonElement>("[data-assisted]")!.click();
  await vi.waitFor(() => expect(host.querySelectorAll("article")).toHaveLength(2));
  const terms = host.querySelector<HTMLTextAreaElement>('[name="librarian-terms"]')!;
  expect(terms.value).toBe("capacity\n容量");
  terms.value = "lease\n retry "; terms.dispatchEvent(new Event("input"));
  expect(host.querySelector<HTMLButtonElement>("article > button")!.disabled).toBe(true);
  inspect.mockResolvedValueOnce(result()); submit();
  await vi.waitFor(() => expect(inspect).toHaveBeenCalledWith(binding.root, binding.project, "core", false, ["lease", "retry"]));
  expect(assist).toHaveBeenCalledTimes(1);
  view.bind({ ...binding, project: "beta" }); expect(terms.value).toBe("");
});
