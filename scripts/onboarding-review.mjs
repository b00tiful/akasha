import { createHash } from "node:crypto";

// Runs only in the offline review document. Canonical parsing and writes stay in Rust.
function reviewApp() {
  const data = JSON.parse(document.getElementById("review-data").textContent);
  const plan = structuredClone(data.plan);
  const make = (tag, text, parent, className) => {
    const node = document.createElement(tag);
    if (text !== undefined) node.textContent = text;
    if (className) node.className = className;
    if (parent) parent.append(node);
    return node;
  };
  const status = document.getElementById("status");
  const catalog = data.packet.sources.flatMap((source, index) => source.excerpts.map((excerpt, offset) => ({
    source: index + 1, excerpt: offset + 1, path: source.path, fingerprint: source.fingerprint,
    line_start: excerpt.line_start, line_end: excerpt.line_end,
    text: new TextDecoder().decode(new TextEncoder().encode(source.text).subarray(excerpt.byte_start, excerpt.byte_end)),
  })));
  let dirty = false;
  const changed = () => { dirty = true; status.textContent = "Unsaved corrections · export a plan to keep your changes"; };
  const updateSummary = () => {
    document.getElementById("summary").textContent = `${plan.notes.length} notes · ${plan.notes.reduce((n, note) => n + note.evidence.length, 0)} claims · ${data.packet.sources.length} source files`;
  };
  const showExcerpt = (source, parent) => {
    make("h4", `${source.path} · lines ${source.line_start}–${source.line_end}`, parent);
    const lines = source.text.split("\n");
    if (lines.at(-1) === "") lines.pop();
    make("pre", lines.map((line, i) => `${source.line_start + i}  ${line}`).join("\n"), parent);
  };
  document.getElementById("project").textContent = data.packet.preparation.project;
  updateSummary();
  const coverage = document.getElementById("coverage");
  make("h2", "Coverage to review", coverage);
  make("p", "This is the original source review. Compile your exported corrections to refresh coverage and metadata checks.", coverage, "muted");
  const listed = data.draft.review.uncited_sources ?? [];
  make("p", listed.length ? `Selected but uncited: ${listed.map(s => typeof s === "string" ? s : s.path).join(", ")}` : "Every selected file is cited. This does not establish complete coverage or factual support.", coverage);
  for (const source of data.draft.review.sampled_sources ?? []) make("pre", JSON.stringify(source, null, 2), coverage);
  for (const window of data.draft.review.uncited_focus_excerpts ?? []) make("pre", JSON.stringify(window, null, 2), coverage);
  if (data.discovery) {
    make("p", `Discovery omitted ${data.discovery.coverage.omitted.length} candidates and excluded ${data.discovery.coverage.excluded_entries} entries.`, coverage);
    const groups = data.discovery.coverage.implementation_groups;
    if (groups?.length) {
      const missing = groups.filter(group => group.selected === 0);
      make("p", `Implementation directories represented: ${groups.length - missing.length}/${groups.length}. Filename groups indicate selected files, not complete component understanding.`, coverage);
      if (missing.length) make("p", `No implementation file selected: ${missing.map(group => group.directory).join(", ")}. Narrow the discovery scopes or prepare a separate batch to inspect these groups.`, coverage);
    }
    const omitted = make("details", undefined, coverage);
    make("summary", "Inspect discovery coverage", omitted);
    make("pre", JSON.stringify(data.discovery.coverage, null, 2), omitted);
  }
  const nav = document.getElementById("notes");
  for (const [n, note] of plan.notes.entries()) {
    const reviewed = data.draft.review.notes[n];
    const slot = data.identityPlan?.slots.find(slot => slot.slot === note.slot);
    const available = slot ? catalog.filter(item => slot.sources.includes(item.source)) : catalog;
    const section = make("section", undefined, document.getElementById("claims"), "note");
    section.id = `note-${n + 1}`;
    const title = slot?.role ?? note.name ?? reviewed.path;
    const link = make("a", title, nav); link.href = `#${section.id}`;
    make("h2", title, section); make("p", reviewed.path, section, "path");
    const metadata = make("details", undefined, section);
    make("summary", "Recorded metadata", metadata);
    make("pre", JSON.stringify(reviewed.fields, null, 2), metadata);
    const controls = make("div", undefined, section, "claim-controls");
    const add = make("button", "Add missing claim", controls); add.type = "button";
    add.setAttribute("aria-label", `Add missing claim to note ${n + 1}`);
    const undo = make("button", "Undo removal", controls); undo.type = "button"; undo.hidden = true;
    undo.setAttribute("aria-label", `Undo claim removal in note ${n + 1}`);
    const count = make("p", undefined, controls, "muted");
    make("p", "Inspect the captured sources below to record omitted behavior. Each note allows 1–4 claims. Remove an unsupported claim before adding one when full. Fresh Rust compilation checks every export.", section, "muted");
    const browser = make("details", undefined, section);
    make("summary", "Browse all captured source excerpts for this note", browser);
    make("p", "Only captured windows in this note's source group are available. Sampling and discovery gaps remain in the original coverage report above. A citation does not prove the claim's meaning.", browser, "muted");
    const sourceButtons = [];
    for (const item of available) {
      const window = make("details", undefined, browser);
      make("summary", `${item.path} · lines ${item.line_start}–${item.line_end}`, window);
      showExcerpt(item, window);
      const button = make("button", "Add claim citing this excerpt", window); button.type = "button";
      button.setAttribute("aria-label", `Add claim to note ${n + 1} citing source ${item.source} excerpt ${item.excerpt}`);
      sourceButtons.push([button, item]);
      button.addEventListener("click", () => addClaim([{ source: item.source, excerpt: item.excerpt }]));
    }
    const cards = make("div", undefined, section);
    let removed;
    const addClaim = sources => {
      if (note.evidence.length >= 4) return;
      note.evidence.push({ kind: "fact", claim: "", sources });
      changed(); renderClaims(note.evidence.length - 1);
    };
    add.addEventListener("click", () => addClaim([]));
    undo.addEventListener("click", () => {
      if (!removed || note.evidence.length >= 4) return;
      const index = removed.index;
      note.evidence.splice(index, 0, removed.claim); removed = undefined;
      changed(); renderClaims(index);
    });
    const renderClaims = focusIndex => {
      cards.replaceChildren();
      count.textContent = `${note.evidence.length}/4 claims`;
      add.disabled = note.evidence.length >= 4;
      for (const [button, item] of sourceButtons) button.disabled = note.evidence.length >= 4 || !item.text.trim();
      undo.hidden = !removed; undo.disabled = note.evidence.length >= 4;
      updateSummary();
      for (const [c, claim] of note.evidence.entries()) {
        const card = make("article", undefined, cards, "claim");
        const edit = make("div", undefined, card, "edit");
        make("h3", `Claim ${c + 1}`, edit);
        const remove = make("button", "Remove claim", edit); remove.type = "button";
        remove.setAttribute("aria-label", `Remove note ${n + 1} claim ${c + 1}`);
        remove.disabled = note.evidence.length === 1;
        remove.addEventListener("click", () => {
          if (note.evidence.length <= 1) return;
          removed = { index: c, claim: note.evidence.splice(c, 1)[0] };
          changed(); renderClaims(Math.min(c, note.evidence.length - 1));
        });
        const kindLabel = make("label", "Evidence type", edit);
        const kind = make("select", undefined, kindLabel);
        kind.setAttribute("aria-label", `Evidence type for note ${n + 1} claim ${c + 1}`);
        for (const value of ["fact", "inference", "unknown"]) { const option = make("option", value, kind); option.value = value; }
        kind.value = claim.kind;
        const textLabel = make("label", "Claim", edit);
        const text = make("textarea", undefined, textLabel); text.value = claim.claim; text.rows = 5;
        text.setAttribute("aria-label", `Claim text for note ${n + 1} claim ${c + 1}`);
        if (c === focusIndex) text.focus();
        text.addEventListener("input", () => { claim.claim = text.value; changed(); });
        const reasonLabel = make("label", "Rationale", edit);
        const reason = make("textarea", undefined, reasonLabel); reason.value = claim.rationale ?? ""; reason.rows = 3;
        reason.setAttribute("aria-label", `Rationale for note ${n + 1} claim ${c + 1}`);
        reasonLabel.hidden = claim.kind === "fact";
        reason.addEventListener("input", () => { claim.rationale = reason.value; changed(); });
        const evidence = make("div", undefined, card, "evidence");
        const chosen = make("div", undefined, evidence);
        const picker = make("details", undefined, evidence);
        make("summary", "Choose source excerpts", picker);
        const checkboxes = [];
        const showSources = () => {
          chosen.replaceChildren();
          if (!claim.sources.length) make("p", "No source selected", chosen, "muted");
          for (const ref of claim.sources) {
            const source = catalog.find(item => item.source === ref.source && item.excerpt === ref.excerpt);
            if (!source) continue;
            showExcerpt(source, chosen);
          }
        };
        for (const item of available) {
          const label = make("label", undefined, picker, "source-option");
          const checkbox = make("input", undefined, label); checkbox.type = "checkbox";
          checkbox.checked = claim.sources.some(ref => ref.source === item.source && ref.excerpt === item.excerpt);
          checkbox.disabled = claim.kind === "unknown";
          make("span", `${item.path} · ${item.line_start}–${item.line_end}`, label);
          checkboxes.push(checkbox);
          checkbox.addEventListener("change", () => {
            claim.sources = available.filter((_, i) => checkboxes[i].checked).map(({ source, excerpt }) => ({ source, excerpt }));
            changed(); showSources();
          });
        }
        kind.addEventListener("change", () => {
          claim.kind = kind.value; reasonLabel.hidden = claim.kind === "fact";
          if (claim.kind === "fact") delete claim.rationale;
          else claim.rationale = reason.value;
          for (const checkbox of checkboxes) checkbox.disabled = claim.kind === "unknown";
          if (claim.kind === "unknown") { claim.sources = []; for (const checkbox of checkboxes) checkbox.checked = false; }
          changed(); showSources();
        });
        showSources();
      }
    };
    renderClaims();
  }
  document.getElementById("export").addEventListener("click", () => {
    const url = URL.createObjectURL(new Blob([JSON.stringify(plan, null, 2) + "\n"], { type: "application/json" }));
    const link = make("a", undefined, document.body); link.href = url; link.download = "reviewed-plan.json"; link.click(); link.remove();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
    dirty = false; status.textContent = "Correction plan exported · compile it to obtain a fresh preview";
  });
  window.addEventListener("beforeunload", event => { if (dirty) { event.preventDefault(); event.returnValue = ""; } });
}

const css = `:root{font-family:system-ui,sans-serif;color:#e5e9e8;background:#101716;color-scheme:dark}*{box-sizing:border-box}body{margin:0}header{padding:28px 4vw;background:#182421;border-bottom:1px solid #344c43}h1{font-size:26px;margin:8px 0}h2{font-size:20px}h3{margin-top:0}h4{margin:0 0 10px;color:#a7d3b8}p{line-height:1.6}button{background:#b4e1bc;color:#15261a;border:0;border-radius:6px;padding:12px 18px;font-weight:700;cursor:pointer}button:focus-visible,a:focus-visible,input:focus-visible,textarea:focus-visible,select:focus-visible{outline:3px solid #c4b27a;outline-offset:3px}.top{display:flex;gap:24px;align-items:center;justify-content:space-between}.kicker{color:#a5bcb0;font-size:12px;letter-spacing:2px;text-transform:uppercase}#status,.muted,.path{color:#aab9b1;font-size:13px}.path{overflow-wrap:anywhere}main{max-width:1440px;margin:auto;padding:24px 4vw 80px}nav{display:flex;flex-wrap:wrap;gap:12px;margin:22px 0}a{color:#b4e1bc}nav a{padding:8px 12px;background:#22342b;border-radius:5px}#coverage{border:1px solid #455144;background:#202b20;padding:4px 20px 12px;border-radius:8px}.note{margin-top:36px;scroll-margin:16px}.claim{display:grid;grid-template-columns:minmax(260px,2fr) minmax(320px,3fr);border:1px solid #34483e;border-radius:8px;margin:18px 0;overflow:hidden}.edit,.evidence{padding:22px}.edit{background:#19251f}.evidence{background:#121c17;min-width:0}label{display:block;font-size:13px;margin-bottom:16px}textarea,select{display:block;width:100%;margin-top:7px;padding:10px;border:1px solid #536258;background:#101914;color:#e5e9e8;border-radius:4px;font:inherit}textarea{resize:vertical;line-height:1.6}pre{overflow:auto;white-space:pre;line-height:1.6;padding:14px;background:#0c130f;border-radius:5px;font-size:12px;max-height:420px}summary{cursor:pointer;font-size:13px;padding:10px 0;color:#bed0c3}.source-option{display:flex;align-items:center;gap:10px;margin:8px 0;overflow-wrap:anywhere}input{accent-color:#b4e1bc}.claim-controls{display:flex;align-items:center;gap:12px;flex-wrap:wrap}.edit>button{margin-bottom:16px}button:disabled{opacity:.45;cursor:default}.steps{margin-top:32px;padding:20px;border-top:1px solid #34483e}[hidden]{display:none!important}@media(max-width:800px){.claim{grid-template-columns:1fr}.top{align-items:flex-start;flex-direction:column}header{padding:22px 5vw}main{padding:18px 5vw}}`;

export function renderOnboardingReview({ packet, plan, draft, identityPlan, discovery }) {
  if (![3, 4, 5].includes(plan.schema_version) || !Array.isArray(plan.notes) || !plan.notes.length
    || !draft.review?.requires_review || draft.review.notes.length !== plan.notes.length) {
    throw new Error("Review requires a compiled excerpt-based plan and its core review report");
  }
  const data = JSON.stringify({ packet, plan, draft, identityPlan, discovery }).replace(/[<>&\u2028\u2029]/gu,
    char => `\\u${char.charCodeAt(0).toString(16).padStart(4, "0")}`);
  const script = `(${reviewApp.toString()})();`;
  const hash = text => createHash("sha256").update(text).digest("base64");
  return `<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'sha256-${hash(script)}'; style-src 'sha256-${hash(css)}'; connect-src 'none'; base-uri 'none'; form-action 'none'">
<title>Akasha · Onboarding review</title><style>${css}</style></head><body>
<header><div class="top"><div><div class="kicker">Akasha / local onboarding</div><h1>Review <span id="project"></span></h1><p id="summary"></p></div><button id="export" type="button">Export correction plan</button></div>
<p>Check each claim against its source. Browse captured sources, add missing claims, remove unsupported claims, and correct wording or citations before exporting.</p><p id="status" role="status">Changes stay in this page until you export a correction plan.</p></header>
<main><aside id="coverage"></aside><nav id="notes" aria-label="Notes"></nav><div id="claims"></div>
<section class="steps"><h2>After review</h2><p>Compile <code>reviewed-plan.json</code> with the saved <code>packet.json</code> to obtain a fresh proposal and preview. Review that preview, then apply it with its full preview ID. Exporting this plan does not publish notes or change their recorded review date.</p></section></main>
<script id="review-data" type="application/json">${data}</script><script>${script}</script></body></html>\n`;
}
