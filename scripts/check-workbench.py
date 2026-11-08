#!/usr/bin/env python3
"""Check temporary-workbench notes: frontmatter, >=2 resolvable wikilinks, whitespace, size budgets.

Usage:
  scripts/check-workbench.py [NOTE ...]   # check the named notes plus orientation budgets
  scripts/check-workbench.py              # notes modified in the last 24 hours plus budgets
  scripts/check-workbench.py --all        # every note (historical notes may predate the rules)

Prints one `path:line: problem` per failure and exits 1 on any failure.
"""
import re
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
VAULT = ROOT / "akasha"
SKIP_DIRS = {"_templates", "evidence", ".trash", ".obsidian"}
# Character budgets for the files every task reads or rewrites (AGENTS.md "Workbench budgets").
BUDGETS = {"00_INDEX.md": 12_000, "Tasks/active.md": 10_000, "Hub.md": 6_000}
PACKET_BUDGET = 30_000  # index + active + last two session entries, as printed by scripts/orient.sh
ENTRY_BUDGET = 1_200  # one appended session log entry (checked for the latest session only)
LINK = re.compile(r"\[\[([^\]|#^]+)(?:[#^][^\]|]*)?(?:\\?\|[^\]]*)?\]\]")
FENCE = re.compile(r"^\s*(```|~~~)")
INLINE_CODE = re.compile(r"`[^`]*`")

failures = []


def fail(path, line, msg):
    failures.append(f"{path.relative_to(ROOT)}:{line}: {msg}")


def notes(paths):
    for p in paths:
        rel = p.relative_to(VAULT).parts
        if p.suffix == ".md" and not SKIP_DIRS.intersection(rel) and p.name != "<date>.md":
            yield p


basenames = {}
for p in VAULT.rglob("*.md"):
    if not SKIP_DIRS.intersection(p.relative_to(VAULT).parts):
        basenames.setdefault(p.stem, []).append(p)


def resolves(target):
    target = target.strip().rstrip("\\")
    candidate = VAULT / target
    if candidate.suffix and candidate.exists():
        return True
    if candidate.with_name(candidate.name + ".md").exists():
        return True
    return "/" not in target and len(basenames.get(target, [])) == 1


def check_note(path):
    text = path.read_text(encoding="utf-8")
    lines = text.splitlines()
    if not lines or lines[0] != "---" or "---" not in lines[1:40]:
        fail(path, 1, "missing or unclosed YAML frontmatter")
    links, in_fence = 0, False
    for n, line in enumerate(lines, 1):
        if line != line.rstrip():
            fail(path, n, "trailing whitespace")
        if FENCE.match(line):
            in_fence = not in_fence
            continue
        if in_fence:
            continue
        for target in LINK.findall(INLINE_CODE.sub("", line)):
            links += 1
            if not resolves(target):
                fail(path, n, f"unresolved wikilink [[{target}]]")
    if links < 2:
        fail(path, 1, f"only {links} outgoing wikilink(s); notes need at least two")


def session_tail(path, entries=2):
    lines = path.read_text(encoding="utf-8").splitlines()
    starts = [i for i, l in enumerate(lines) if re.match(r"^## \d\d:\d\d ", l)]
    return lines, starts, (starts[-entries] if len(starts) >= entries else 0)


def check_budgets():
    packet = 0
    for rel, limit in BUDGETS.items():
        p = VAULT / rel
        size = len(p.read_text(encoding="utf-8"))
        if rel != "Hub.md":
            packet += size
        if size > limit:
            fail(p, 1, f"{size:,} characters exceeds the {limit:,} budget; archive history, keep current state")
    sessions = sorted(VAULT.glob("Sessions/20[0-9][0-9]-[0-9][0-9]-[0-9][0-9].md"))
    if sessions:
        lines, starts, begin = session_tail(sessions[-1])
        packet += len("\n".join(lines[begin:]))
        if starts:
            last = "\n".join(lines[starts[-1]:])
            if len(last) > ENTRY_BUDGET:
                fail(sessions[-1], starts[-1] + 1,
                     f"latest log entry is {len(last):,} characters (budget {ENTRY_BUDGET:,}); move detail to the owning note")
    if packet > PACKET_BUDGET:
        fail(VAULT / "00_INDEX.md", 1, f"orientation packet {packet:,} characters exceeds {PACKET_BUDGET:,}")
    return packet


def main(argv):
    if "--all" in argv:
        targets = list(notes(VAULT.rglob("*.md")))
    elif argv:
        targets = list(notes(Path(a).resolve() for a in argv))
    else:
        cutoff = time.time() - 24 * 3600
        targets = [p for p in notes(VAULT.rglob("*.md")) if p.stat().st_mtime >= cutoff]
    for p in targets:
        check_note(p)
    packet = check_budgets()
    for f in failures:
        print(f)
    status = "FAIL" if failures else "ok"
    print(f"workbench check {status}: {len(targets)} notes, {len(failures)} problem(s), orientation packet {packet:,} chars",
          file=sys.stderr if failures else sys.stdout)
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
