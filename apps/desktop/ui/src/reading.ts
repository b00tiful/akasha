import type { LibraryDocumentLink } from "./types";

export function linkTitle(link: LibraryDocumentLink): string {
  return link.kind === "canonical"
    ? `Open ${link.target_id}`
    : `${link.target_id} — ${link.kind === "projection" ? "Index or roadmap" : "This Markdown file"} is not a library document`;
}

export function renderReading(
  host: HTMLElement, source: string, links: readonly LibraryDocumentLink[] = [],
  onActivate: (link: LibraryDocumentLink) => void = () => {},
): void {
  host.replaceChildren();
  const lines = source.split(/\r?\n/u);
  const offsets: number[] = [];
  let position = 0;
  for (const line of source.split(/(?<=\n)/u)) {
    offsets.push(position);
    position += line.length;
  }
  let index = 0;

  if (lines[0] === "---") {
    const closing = lines.indexOf("---", 1);
    if (closing >= 0) {
      const frontmatter = document.createElement("pre");
      frontmatter.className = "reading-frontmatter";
      frontmatter.textContent = lines.slice(0, closing + 1).join("\n");
      host.append(frontmatter);
      index = closing + 1;
    }
  }

  while (index < lines.length) {
    const line = lines[index] ?? "";
    const fence = /^ {0,3}(`{3,}|~{3,})/u.exec(line);
    if (fence?.[1]) {
      const marker = fence[1][0] ?? "`";
      const minimum = fence[1].length;
      const block = [line];
      index += 1;
      while (index < lines.length) {
        const next = lines[index] ?? "";
        block.push(next);
        index += 1;
        const closing = new RegExp(`^ {0,3}${escapeRegExp(marker)}{${minimum},}\\s*$`, "u");
        if (closing.test(next)) {
          break;
        }
      }
      const pre = document.createElement("pre");
      const code = document.createElement("code");
      code.textContent = block.join("\n");
      pre.append(code);
      host.append(pre);
      continue;
    }

    if (line.trim().length === 0) {
      index += 1;
      continue;
    }
    const heading = /^(#{1,6})\s+(.+)$/u.exec(line);
    const element = heading?.[1]
      ? document.createElement(`h${heading[1].length}`)
      : document.createElement("p");
    const text = heading?.[2] ?? line;
    const offset = (offsets[index] ?? source.length) + line.length - text.length;
    appendInline(element, text, offset, links, onActivate);
    host.append(element);
    index += 1;
  }
}

function appendInline(
  host: HTMLElement, source: string, start: number, links: readonly LibraryDocumentLink[],
  onActivate: (link: LibraryDocumentLink) => void,
): void {
  const strongMatches = [...source.matchAll(/\*\*[^*\n]+\*\*/gu)];
  const tokens = [
    ...strongMatches.map((match) => ({ from: match.index!, to: match.index! + match[0].length, link: null })),
    ...links.filter((link) => link.from >= start && link.to <= start + source.length)
      .map((link) => ({ from: link.from - start, to: link.to - start, link })),
  ].sort((a, b) => a.from - b.from || b.to - a.to);
  let offset = 0;
  for (const token of tokens) {
    if (token.from < offset) continue;
    host.append(document.createTextNode(source.slice(offset, token.from)));
    if (!token.link) {
      const strong = document.createElement("strong");
      appendInline(strong, source.slice(token.from + 2, token.to - 2), start + token.from + 2, links, onActivate);
      host.append(strong);
    } else {
      const binding = token.link;
      const element = document.createElement("button");
      element.type = "button";
      element.className = "reading-wikilink";
      element.textContent = source.slice(binding.label_from - start, binding.label_to - start);
      element.title = linkTitle(binding);
      element.dataset.documentLink = String(binding.from);
      element.setAttribute("role", "link");
      element.setAttribute("aria-label", linkTitle(binding));
      element.setAttribute("aria-disabled", String(binding.kind !== "canonical"));
      element.addEventListener("click", () => onActivate(binding));
      host.append(element);
    }
    offset = token.to;
  }
  host.append(document.createTextNode(source.slice(offset)));
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&");
}
