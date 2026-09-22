import { ORBIT_PAGE_SIZE, SKY_PAGE_SIZE } from "./local-akasha-layout";
import type { LocalAkashaModel } from "./local-akasha-model";
import type { LocalNavigation } from "./local-akasha-scene";
import type { LibraryBook, LibraryCategory, LocalNavigationState } from "./types";

export interface RestoredLocalNavigation {
  navigation: LocalNavigation;
  note: LibraryBook | null;
}

export function emptyLocalNavigation(): LocalNavigation {
  return { section: null, skyPage: 0, pages: {} };
}

export function restoreLocalNavigation(
  model: LocalAkashaModel,
  state: LocalNavigationState | null,
): RestoredLocalNavigation {
  const navigation = emptyLocalNavigation();
  if (!state || state.version !== 1 || state.root !== model.root || state.project !== model.project) {
    return { navigation, note: null };
  }

  const sections = sortedSections(model);
  const sectionById = new Map(sections.map((section) => [section.note_type, section]));
  const skyAnchor = state.sky_anchor ? sectionById.get(state.sky_anchor) : undefined;
  if (skyAnchor) {
    navigation.skyPage = Math.floor(sections.indexOf(skyAnchor) / SKY_PAGE_SIZE);
  }

  for (const [sectionId, noteId] of Object.entries(state.page_anchors)) {
    const section = sectionById.get(sectionId);
    const noteIndex = section?.books.findIndex((book) => book.id === noteId) ?? -1;
    if (section && noteIndex >= 0) {
      navigation.pages[sectionId] = Math.floor(noteIndex / ORBIT_PAGE_SIZE);
    }
  }

  if (state.section && sectionById.has(state.section)) {
    navigation.section = state.section;
    if (!skyAnchor) {
      navigation.skyPage = Math.floor(
        sections.findIndex((section) => section.note_type === state.section) / SKY_PAGE_SIZE,
      );
    }
  }

  const note = state.note
    ? sections.flatMap((section) => section.books).find((book) => book.id === state.note) ?? null
    : null;
  if (note) {
    const section = sectionById.get(note.note_type);
    const noteIndex = section?.books.findIndex((book) => book.id === note.id) ?? -1;
    if (section && noteIndex >= 0) {
      navigation.section = section.note_type;
      navigation.skyPage = Math.floor(sections.indexOf(section) / SKY_PAGE_SIZE);
      navigation.pages[section.note_type] = Math.floor(noteIndex / ORBIT_PAGE_SIZE);
    }
  }

  return { navigation, note };
}

export function snapshotLocalNavigation(
  model: LocalAkashaModel,
  navigation: LocalNavigation,
  selectedId: string | null,
): LocalNavigationState {
  const sections = sortedSections(model);
  const sectionById = new Map(sections.map((section) => [section.note_type, section]));
  const skyPage = boundedPage(navigation.skyPage, sections.length, SKY_PAGE_SIZE);
  const pageAnchors: Record<string, string> = {};

  for (const [sectionId, page] of Object.entries(navigation.pages)) {
    const section = sectionById.get(sectionId);
    if (!section?.books.length) continue;
    const bounded = boundedPage(page, section.books.length, ORBIT_PAGE_SIZE);
    const anchor = section.books[bounded * ORBIT_PAGE_SIZE];
    if (anchor) pageAnchors[sectionId] = anchor.id;
  }

  const note = selectedId
    ? sections.flatMap((section) => section.books).find((book) => book.id === selectedId) ?? null
    : null;
  const section = note?.note_type ?? (navigation.section && sectionById.has(navigation.section)
    ? navigation.section
    : null);

  return {
    version: 1,
    root: model.root,
    project: model.project,
    section,
    sky_anchor: sections[skyPage * SKY_PAGE_SIZE]?.note_type ?? null,
    page_anchors: pageAnchors,
    note: note?.id ?? null,
  };
}

function sortedSections(model: LocalAkashaModel): LibraryCategory[] {
  return [...model.sections].sort((a, b) =>
    a.note_type < b.note_type ? -1 : a.note_type > b.note_type ? 1 : 0
  );
}

function boundedPage(page: number, total: number, size: number): number {
  const maximum = Math.max(0, Math.ceil(total / size) - 1);
  if (!Number.isSafeInteger(page)) return 0;
  return Math.min(Math.max(0, page), maximum);
}
