import "./local-akasha.css";
import { LOCAL_TRAVEL_MS, ORBIT_PAGE_SIZE, SKY_PAGE_SIZE, hashSky, orbitLayout, sectionLayout, type SkyPoint } from "./local-akasha-layout";
import type { LocalAkashaModel } from "./local-akasha-model";
import { mountLocalSky } from "./local-akasha-sky";
import type { LibraryBook, LibraryCategory } from "./types";

export interface LocalNavigation {
  section: string | null;
  skyPage: number;
  pages: Record<string, number>;
}
type Phase = "directory" | "entering-section" | "section" | "note-open" | "leaving-section";
export interface LocalSceneHandle {
  destroy(): void;
  select(id: string | null): void;
  back(): void;
  setReducedMotion(value: boolean): void;
}

export function mountLocalAkasha(
  host: HTMLElement,
  model: LocalAkashaModel,
  navigation: LocalNavigation,
  reducedMotion: boolean,
  callbacks: { canNavigate(): boolean; onSelect(book: LibraryBook): void; onBack(): void },
): LocalSceneHandle {
  const root = document.createElement("section");
  root.className = "local-akasha";
  root.setAttribute("aria-label", `Local Akasha, project ${model.project}`);
  root.innerHTML = `<canvas class="local-sky" aria-hidden="true"></canvas>
    <nav class="local-breadcrumb" aria-label="Local navigation"><button type="button">← Directory sky</button><span></span></nav>
    <div class="local-map"><div class="local-directory"></div><div class="local-system" hidden></div></div>
    <footer class="local-footer"><span class="local-count" role="status"></span><div class="local-pages"></div></footer>`;
  host.replaceChildren(root);
  const get = <T extends HTMLElement>(selector: string) => root.querySelector<T>(selector)!;
  const directory = get(".local-directory");
  const system = get(".local-system");
  const map = get(".local-map");
  const crumb = get(".local-breadcrumb span");
  const backButton = get<HTMLButtonElement>(".local-breadcrumb button");
  const pages = get(".local-pages");
  const count = get(".local-count");
  crumb.textContent = `Project ${model.project} · vault ${model.root}`;
  crumb.title = crumb.textContent;
  const sky = mountLocalSky(get<HTMLCanvasElement>("canvas"), model.key, reducedMotion);
  root.style.setProperty("--local-travel-duration", `${LOCAL_TRAVEL_MS}ms`);
  const positions = sectionLayout(model.key, model.sections.map((section) => section.note_type));
  const sections = [...model.sections].sort((a, b) => a.note_type < b.note_type ? -1 : a.note_type > b.note_type ? 1 : 0);
  let phase: Phase = "directory";
  let selectedId: string | null = null;
  let lastSelectedId: string | null = null;
  let transition: ReturnType<typeof setTimeout> | undefined;
  let finishTransition: (() => void) | null = null;
  let destroyed = false;

  function setPhase(value: Phase): void {
    phase = value;
    root.dataset.phase = value;
  }
  function activeSection(): LibraryCategory | undefined {
    return sections.find((section) => section.note_type === navigation.section);
  }
  function button(label: string, action: () => void): HTMLButtonElement {
    const node = document.createElement("button");
    node.type = "button";
    node.textContent = label;
    node.addEventListener("click", action);
    return node;
  }
  function controls(page: number, total: number, size: number, change: (page: number) => void): void {
    pages.replaceChildren();
    if (total <= size) return;
    const previous = button("←", () => change(page - 1));
    previous.disabled = page === 0;
    previous.setAttribute("aria-label", "Previous orbital page");
    const next = button("→", () => change(page + 1));
    next.disabled = (page + 1) * size >= total;
    next.setAttribute("aria-label", "Next orbital page");
    const label = document.createElement("span");
    label.textContent = `${page + 1} / ${Math.ceil(total / size)}`;
    pages.append(previous, label, next);
  }
  function showDirectory(focusId?: string): void {
    setPhase("directory");
    system.hidden = true;
    directory.hidden = false;
    directory.inert = false;
    directory.style.transform = "";
    directory.replaceChildren();
    backButton.hidden = true;
    const maxPage = Math.max(0, Math.ceil(sections.length / SKY_PAGE_SIZE) - 1);
    navigation.skyPage = Math.min(Math.max(0, navigation.skyPage), maxPage);
    for (const section of sections.slice(navigation.skyPage * SKY_PAGE_SIZE, (navigation.skyPage + 1) * SKY_PAGE_SIZE)) {
      const node = button("", () => enter(section));
      node.className = "local-section";
      node.dataset.section = section.note_type;
      node.setAttribute("aria-label", `${section.note_type} section, ${section.books.length} notes`);
      node.innerHTML = `<span class="local-star" aria-hidden="true">✦</span><span class="local-label"></span><small></small>`;
      node.querySelector(".local-label")!.textContent = section.note_type;
      node.querySelector("small")!.textContent = `${section.books.length} notes`;
      const point = positions.get(section.note_type)!;
      node.style.left = `${point.x * 100}%`;
      node.style.top = `${point.y * 100}%`;
      node.style.setProperty("--pulse-delay", `${-(hashSky(section.note_type) % 9000)}ms`);
      node.style.setProperty("--pulse-duration", `${5 + hashSky(section.note_type) % 5}s`);
      node.title = section.note_type;
      directory.append(node);
      if (section.note_type === focusId) node.focus();
    }
    if (!sections.length) directory.textContent = "No configured sections in this project.";
    count.textContent = `${sections.length} sections · ${model.noteCount} notes`;
    controls(navigation.skyPage, sections.length, SKY_PAGE_SIZE, (page) => {
      navigation.skyPage = page;
      showDirectory();
      directory.querySelector<HTMLButtonElement>("button")?.focus();
    });
  }
  function showSection(focus = false): void {
    const section = activeSection();
    if (!section) { navigation.section = null; showDirectory(); return; }
    directory.hidden = true;
    system.hidden = false;
    system.inert = selectedId !== null;
    backButton.hidden = false;
    system.replaceChildren();
    setPhase(selectedId ? "note-open" : "section");
    const page = Math.min(navigation.pages[section.note_type] ?? 0, Math.max(0, Math.ceil(section.books.length / ORBIT_PAGE_SIZE) - 1));
    navigation.pages[section.note_type] = page;
    const books = section.books.slice(page * ORBIT_PAGE_SIZE, (page + 1) * ORBIT_PAGE_SIZE);
    for (let ring = 0; ring < Math.ceil(books.length / 8); ring++) {
      const circle = document.createElement("div");
      circle.className = `local-orbit local-orbit-${ring}`;
      circle.setAttribute("aria-hidden", "true");
      system.append(circle);
    }
    const center = document.createElement("div");
    center.className = "local-center";
    center.innerHTML = `<span class="local-star" aria-hidden="true">✦</span><strong></strong><small></small>`;
    center.querySelector("strong")!.textContent = section.note_type;
    center.querySelector("small")!.textContent = `${section.books.length} notes`;
    system.append(center);
    const points = orbitLayout(books.map((book) => book.id));
    for (const [index, book] of books.entries()) {
      const node = button("", () => {
        if (phase !== "entering-section" && phase !== "leaving-section") callbacks.onSelect(book);
      });
      node.className = "local-note";
      node.dataset.note = book.id;
      node.setAttribute("aria-label", `${book.label}, ${book.note_type}, Markdown note`);
      node.setAttribute("aria-pressed", String(book.id === selectedId));
      node.innerHTML = `<span aria-hidden="true">▤</span><span class="local-note-label"></span>`;
      node.querySelector(".local-note-label")!.textContent = book.label;
      node.title = `${book.label}\n${book.id}`;
      const point = points.get(book.id)!;
      node.style.left = `${point.x * 100}%`;
      node.style.top = `${point.y * 100}%`;
      node.style.setProperty("--emerge-delay", `${520 + index * 28}ms`);
      system.append(node);
    }
    const changePage = (next: number) => {
      if (!callbacks.canNavigate()) return;
      navigation.pages[section.note_type] = next;
      showSection(true);
    };
    const remaining = section.books.length - (page + 1) * ORBIT_PAGE_SIZE;
    if (remaining > 0) {
      const cluster = button(`+${remaining}`, () => changePage(page + 1));
      cluster.className = "local-cluster";
      cluster.setAttribute("aria-label", `${remaining} more notes, show next orbital page`);
      system.append(cluster);
    }
    if (!books.length) {
      const empty = document.createElement("p");
      empty.className = "local-empty";
      empty.textContent = "This section has no notes yet.";
      system.append(empty);
    }
    count.textContent = `${section.note_type} · ${books.length ? page * ORBIT_PAGE_SIZE + 1 : 0}–${page * ORBIT_PAGE_SIZE + books.length} of ${section.books.length} notes`;
    controls(page, section.books.length, ORBIT_PAGE_SIZE, changePage);
    if (focus) (system.querySelector<HTMLButtonElement>(".local-note") ?? backButton).focus();
  }
  function travel(done: () => void): void {
    clearTimeout(transition);
    finishTransition = () => { finishTransition = null; if (!destroyed) done(); };
    if (reducedMotion) finishTransition();
    else transition = setTimeout(() => { sky.settle(); finishTransition?.(); }, LOCAL_TRAVEL_MS);
  }
  function stagePoint(point: SkyPoint): SkyPoint {
    const rect = root.getBoundingClientRect();
    const field = map.getBoundingClientRect();
    return { x: (field.x - rect.x + point.x * field.width) / (rect.width || 1),
      y: (field.y - rect.y + point.y * field.height) / (rect.height || 1) };
  }
  function enter(section: LibraryCategory): void {
    if (phase !== "directory" || !callbacks.canNavigate()) return;
    const star = [...directory.querySelectorAll<HTMLElement>(".local-section")]
      .find((node) => node.dataset.section === section.note_type)?.querySelector(".local-star")?.getBoundingClientRect();
    const bounds = root.getBoundingClientRect();
    const source = star && bounds.width ? { x: (star.x + star.width / 2 - bounds.x) / bounds.width,
      y: (star.y + star.height / 2 - bounds.y) / bounds.height } : stagePoint(positions.get(section.note_type)!);
    navigation.section = section.note_type;
    showSection();
    directory.hidden = false;
    directory.inert = true;
    system.inert = true;
    const point = positions.get(section.note_type)!;
    directory.style.transformOrigin = `${point.x * 100}% ${point.y * 100}%`;
    directory.style.transform = `translate(${(0.5 - point.x) * 100}%, ${(0.5 - point.y) * 100}%) scale(2.4)`;
    setPhase("entering-section");
    sky.travel(source, stagePoint({ x: .5, y: .5 }), false);
    travel(() => showSection(true));
  }
  function back(): void {
    if (phase === "entering-section" || phase === "leaving-section" || phase === "directory") return;
    if (!callbacks.canNavigate()) return;
    const previous = navigation.section;
    selectedId = null;
    navigation.section = null;
    root.classList.remove("has-note");
    system.inert = true;
    directory.hidden = false;
    directory.inert = true;
    setPhase("leaving-section");
    sky.travel(stagePoint({ x: .5, y: .5 }), stagePoint(positions.get(previous!) ?? { x: .5, y: .5 }), true);
    // Resolve the stored approach transform before applying its inverse.
    void directory.offsetWidth;
    directory.style.transform = "";
    travel(() => showDirectory(previous ?? undefined));
  }
  backButton.addEventListener("click", callbacks.onBack);
  map.addEventListener("keydown", (event) => {
    if (!(event.target instanceof HTMLButtonElement)) return;
    const direction = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] }[event.key];
    if (!direction) return;
    const current = event.target.getBoundingClientRect();
    const nodes = [...(phase === "directory" ? directory : system).querySelectorAll<HTMLButtonElement>("button")];
    const next = nodes.filter((node) => node !== event.target).map((node) => {
      const box = node.getBoundingClientRect();
      const dx = box.x - current.x; const dy = box.y - current.y;
      const forward = dx * direction[0]! + dy * direction[1]!;
      const lateral = Math.abs(dx * direction[1]! - dy * direction[0]!);
      return { node, forward, score: Math.hypot(dx, dy) + lateral * 2 };
    }).filter((item) => item.forward > 1).sort((a, b) => a.score - b.score)[0];
    event.preventDefault();
    next?.node.focus();
  });
  root.classList.toggle("local-reduced-motion", reducedMotion);
  showDirectory();
  if (navigation.section) showSection();
  return {
    destroy() { destroyed = true; clearTimeout(transition); sky.destroy(); root.remove(); },
    back,
    setReducedMotion(value) {
      reducedMotion = value;
      root.classList.toggle("local-reduced-motion", value);
      sky.setReducedMotion(value);
      if (value && finishTransition) { clearTimeout(transition); finishTransition(); }
    },
    select(id) {
      clearTimeout(transition);
      finishTransition = null;
      sky.settle();
      sky.setPaused(id !== null);
      if (id) lastSelectedId = id;
      selectedId = id;
      root.classList.toggle("has-note", id !== null);
      if (id) {
        const section = sections.find((item) => item.books.some((book) => book.id === id));
        if (section) {
          navigation.section = section.note_type;
          navigation.pages[section.note_type] = Math.floor(section.books.findIndex((book) => book.id === id) / ORBIT_PAGE_SIZE);
        }
      }
      if (navigation.section) {
        showSection();
        if (!id) [...system.querySelectorAll<HTMLButtonElement>(".local-note")]
          .find((node) => node.dataset.note === lastSelectedId)?.focus();
      }
    },
  };
}
