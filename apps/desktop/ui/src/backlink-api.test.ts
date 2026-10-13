import { expect, it, vi } from "vitest";
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn(async () => undefined) }));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
import { followDocumentBacklink, inspectDocumentBacklinks } from "./api";

it("passes exact baseline and identities over the read-only backlink IPCs", async () => {
  const id = "Projects/alpha/entities/🌘.md", source = "🌘\r\nExact bytes\r\n";
  await inspectDocumentBacklinks(" /root ", " alpha ", id, source, 30);
  expect(invoke).toHaveBeenLastCalledWith("inspect_document_backlinks", {
    root: "/root", project: "alpha", id, expectedSource: source, limit: 30,
  });
  const sourceId = "Global/entities/Кириллица.md";
  await followDocumentBacklink("/root", "alpha", id, source, sourceId, 100);
  expect(invoke).toHaveBeenLastCalledWith("follow_document_backlink", {
    root: "/root", project: "alpha", id, expectedSource: source, sourceId, limit: 100,
  });
});
