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

    const heading = /^(#{1,6})\s+/u.exec(line);
    if (heading?.[0]) {
      decorations.push({ from: lineStart, to: lineStart + heading[0].length, kind: "syntax" });
      if (lineEnd > lineStart + heading[0].length) {
        decorations.push({
          from: lineStart + heading[0].length,
          to: lineEnd,
          kind: "heading",
        });
      }
    }

    collectStrong(line, lineStart, decorations);
    collectWikilinks(line, lineStart, decorations);
    offset += rawLine.length;
  }

  return decorations.sort((left, right) => left.from - right.from || left.to - right.to);
}

function collectStrong(
  line: string,
  lineStart: number,
  output: PreviewDecoration[],
): void {
  for (const match of line.matchAll(/\*\*([^*\n]+)\*\*/gu)) {
    if (match.index === undefined || !match[1]) {
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
  output: PreviewDecoration[],
): void {
  for (const match of line.matchAll(/!?\[\[([^\]\n]+)\]\]/gu)) {
    if (match.index === undefined || !match[0] || !match[1]) {
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
