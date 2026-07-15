export type PreviewDecorationKind = "syntax" | "heading" | "strong" | "wikilink";

export interface PreviewDecoration {
  from: number;
  to: number;
  kind: PreviewDecorationKind;
}

export function previewDecorations(source: string, cursor: number): PreviewDecoration[] {
  const decorations: PreviewDecoration[] = [];
  const lines = source.split(/(?<=\n)/u);
  let offset = 0;
  let frontmatter = source.startsWith("---\n") || source.startsWith("---\r\n");
  let fence: { marker: string; length: number } | null = null;
  let comment: CommentKind | null = null;

  for (const rawLine of lines) {
    const line = rawLine.replace(/\r?\n$/u, "");
    const lineStart = offset;
    const lineEnd = lineStart + line.length;
    const active = cursor >= lineStart && cursor <= lineEnd;

    if (frontmatter) {
      if (lineStart > 0 && line === "---") {
        frontmatter = false;
      }
      offset += rawLine.length;
      continue;
    }

    const fenceMatch = /^ {0,3}(`{3,}|~{3,})/u.exec(line);
    if (fenceMatch?.[1]) {
      const marker = fenceMatch[1].slice(0, 1);
      if (!fence) {
        fence = { marker, length: fenceMatch[1].length };
      } else if (fence.marker === marker && fenceMatch[1].length >= fence.length) {
        fence = null;
      }
      offset += rawLine.length;
      continue;
    }
    if (fence || active) {
      offset += rawLine.length;
      continue;
    }

    const protectedSource = protectedRanges(line, comment);
    comment = protectedSource.comment;

    const heading = /^(#{1,6})\s+/u.exec(line);
    if (heading?.[0] && !overlapsProtected(0, heading[0].length, protectedSource.ranges)) {
      decorations.push({ from: lineStart, to: lineStart + heading[0].length, kind: "syntax" });
      if (lineEnd > lineStart + heading[0].length) {
        decorations.push({
          from: lineStart + heading[0].length,
          to: lineEnd,
          kind: "heading",
        });
      }
    }

    collectStrong(line, lineStart, protectedSource.ranges, decorations);
    collectWikilinks(line, lineStart, protectedSource.ranges, decorations);
    offset += rawLine.length;
  }

  return decorations.sort((left, right) => left.from - right.from || left.to - right.to);
}

function collectStrong(
  line: string,
  lineStart: number,
  protectedSource: SourceRange[],
  output: PreviewDecoration[],
): void {
  for (const match of line.matchAll(/\*\*([^*\n]+)\*\*/gu)) {
    if (match.index === undefined || !match[1]) {
      continue;
    }
    if (
      isEscaped(line, match.index) ||
      overlapsProtected(match.index, match.index + match[0].length, protectedSource)
    ) {
      continue;
    }
    const start = lineStart + match.index;
    output.push({ from: start, to: start + 2, kind: "syntax" });
    output.push({ from: start + 2, to: start + 2 + match[1].length, kind: "strong" });
    output.push({
      from: start + 2 + match[1].length,
      to: start + 4 + match[1].length,
      kind: "syntax",
    });
  }
}

function collectWikilinks(
  line: string,
  lineStart: number,
  protectedSource: SourceRange[],
  output: PreviewDecoration[],
): void {
  for (const match of line.matchAll(/!?\[\[([^\]\n]+)\]\]/gu)) {
    if (match.index === undefined || !match[0] || !match[1]) {
      continue;
    }
    if (
      isEscaped(line, match.index) ||
      overlapsProtected(match.index, match.index + match[0].length, protectedSource)
    ) {
      continue;
    }
    const aliasSeparator = match[1].indexOf("|");
    const labelOffset = aliasSeparator >= 0 ? aliasSeparator + 1 : 0;
    const openerLength = match[0].startsWith("!") ? 3 : 2;
    const start = lineStart + match.index;
    const visibleStart = start + openerLength + labelOffset;
    const visibleEnd = start + match[0].length - 2;
    output.push({ from: start, to: visibleStart, kind: "syntax" });
    output.push({ from: visibleStart, to: visibleEnd, kind: "wikilink" });
    output.push({ from: visibleEnd, to: start + match[0].length, kind: "syntax" });
  }
}

type CommentKind = "html" | "obsidian";

interface SourceRange {
  from: number;
  to: number;
}

function protectedRanges(
  line: string,
  initialComment: CommentKind | null,
): { ranges: SourceRange[]; comment: CommentKind | null } {
  const ranges: SourceRange[] = [];
  let comment = initialComment;
  let offset = 0;
  if (comment) {
    const closing = comment === "html" ? "-->" : "%%";
    const close = line.indexOf(closing);
    if (close < 0) {
      return { ranges: [{ from: 0, to: line.length }], comment };
    }
    offset = close + closing.length;
    ranges.push({ from: 0, to: offset });
    comment = null;
  }

  while (offset < line.length) {
    const html = line.indexOf("<!--", offset);
    const obsidian = line.indexOf("%%", offset);
    const candidates = [
      html >= 0 ? { index: html, kind: "html" as const } : null,
      obsidian >= 0 ? { index: obsidian, kind: "obsidian" as const } : null,
    ].filter((candidate): candidate is { index: number; kind: CommentKind } => candidate !== null);
    candidates.sort((left, right) => left.index - right.index);
    const next = candidates[0];
    if (!next) {
      break;
    }
    const closing = next.kind === "html" ? "-->" : "%%";
    const openingLength = next.kind === "html" ? 4 : 2;
    const close = line.indexOf(closing, next.index + openingLength);
    if (close < 0) {
      ranges.push({ from: next.index, to: line.length });
      return { ranges, comment: next.kind };
    }
    offset = close + closing.length;
    ranges.push({ from: next.index, to: offset });
  }

  let index = 0;
  while (index < line.length) {
    if (line[index] !== "`" || isEscaped(line, index)) {
      index += 1;
      continue;
    }
    let length = 1;
    while (line[index + length] === "`") {
      length += 1;
    }
    const marker = "`".repeat(length);
    const close = line.indexOf(marker, index + length);
    if (close < 0) {
      index += length;
      continue;
    }
    const end = close + length;
    if (!overlapsProtected(index, end, ranges)) {
      ranges.push({ from: index, to: end });
    }
    index = end;
  }
  ranges.sort((left, right) => left.from - right.from || left.to - right.to);
  return { ranges, comment };
}

function overlapsProtected(from: number, to: number, ranges: SourceRange[]): boolean {
  return ranges.some((range) => from < range.to && to > range.from);
}

function isEscaped(source: string, index: number): boolean {
  let backslashes = 0;
  for (let cursor = index - 1; cursor >= 0 && source[cursor] === "\\"; cursor -= 1) {
    backslashes += 1;
  }
  return backslashes % 2 === 1;
}
