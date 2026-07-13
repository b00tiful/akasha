export function renderReading(host: HTMLElement, source: string): void {
  host.replaceChildren();
  const lines = source.split(/\r?\n/u);
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
    appendInline(element, heading?.[2] ?? line);
    host.append(element);
    index += 1;
  }
}

function appendInline(host: HTMLElement, source: string): void {
  const pattern = /(\*\*[^*\n]+\*\*|!?\[\[[^\]\n]+\]\])/gu;
  let offset = 0;
  for (const match of source.matchAll(pattern)) {
    if (match.index === undefined || !match[0]) {
      continue;
    }
    host.append(document.createTextNode(source.slice(offset, match.index)));
    if (match[0].startsWith("**")) {
      const strong = document.createElement("strong");
      strong.textContent = match[0].slice(2, -2);
      host.append(strong);
    } else {
      const link = document.createElement("span");
      link.className = "reading-wikilink";
      const content = match[0].replace(/^!?\[\[/u, "").slice(0, -2);
      const [target, alias] = content.split("|", 2);
      link.textContent = alias ?? target ?? content;
      link.title = target ?? content;
      host.append(link);
    }
    offset = match.index + match[0].length;
  }
  host.append(document.createTextNode(source.slice(offset)));
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&");
}
