import { invoke } from "@tauri-apps/api/core";

// Developer-only acceptance build; normal builds eliminate this probe and its fixture path.
export const recoveryProbeEnabled = import.meta.env.VITE_AKASHA_RECOVERY_PROBE === "1";
export const recoveryProbeRoot = import.meta.env.VITE_AKASHA_RECOVERY_PROBE_ROOT ?? "";
let started = false;

function panelChecks(): Record<string, boolean> {
  const panel = document.querySelector<HTMLElement>("#inventory-panel")!;
  const fallback = document.querySelector<HTMLElement>("#fallback")!;
  const bounds = panel.getBoundingClientRect();
  return {
    refusal: document.querySelector("#status")!.textContent!.startsWith("Recovery inspection needed:"),
    panelVisible: !panel.hidden && bounds.width > 0 && bounds.height > 0,
    panelInViewport: bounds.left >= 0 && bounds.top >= 0 &&
      bounds.right <= innerWidth + 1 && bounds.bottom <= innerHeight + 1,
    title: document.querySelector("#inventory-title")!.textContent === "Recovery inspection",
    exactJournalPath: fallback.textContent!.includes(
      `Pending journal: ${recoveryProbeRoot}/Projects/example/.akasha-edit-journal.json`,
    ),
    guidance: fallback.textContent!.includes("Back up the project") &&
      fallback.textContent!.includes("Keep the journal"),
    dashboardHidden: Boolean(document.querySelector<HTMLElement>("#dashboard")!.hidden),
    noteHidden: Boolean(document.querySelector<HTMLElement>("#note-overlay")!.hidden),
    noScene: document.querySelector("#scene")!.childElementCount === 0,
    noBooks: fallback.querySelector("button, a") === null,
  };
}

export function scheduleRecoveryProbe(): void {
  if (!recoveryProbeEnabled || started) return;
  started = true;
  window.setTimeout(() => void runRecoveryProbe(), 0);
}

async function runRecoveryProbe(): Promise<void> {
  const initial = panelChecks();
  const form = document.querySelector<HTMLFormElement>("#library-form")!;
  form.requestSubmit();
  const deadline = performance.now() + 8_000;
  while (form.classList.contains("is-loading") && performance.now() < deadline) {
    await new Promise((resolve) => window.setTimeout(resolve, 25));
  }
  const retry = panelChecks();
  const passed = !form.classList.contains("is-loading") &&
    Object.values(initial).every(Boolean) && Object.values(retry).every(Boolean);
  const report = {
    version: 1, passed, initial, retry,
    viewport: { width: innerWidth, height: innerHeight },
  };
  document.title = `AKASHA_RECOVERY_${passed ? "PASS" : "FAIL"}`;
  await invoke("write_runtime_probe_report", { report: JSON.stringify(report) });
}
