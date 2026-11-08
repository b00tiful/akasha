#!/usr/bin/env python3
# Usage: scripts/simulate-context-growth.py SCRATCH_DIR [AKASHA_BINARY]
# Copies tests/fixtures/resolution into SCRATCH_DIR/sim and grows it for 26 weeks through the
# real CLI, printing which `akasha context` sections survive the default budget. Never touches
# a real data root.
"""Grow a disposable Akasha project through the real CLI and snapshot `akasha context` composition.

Models ordinary use: tasks opened and closed weekly with progress notes, occasional problems,
one entity per week (spec-compliant one-line index rows), three sessions and one handoff per week,
and a roadmap that keeps one line per record plus a short milestone paragraph every four weeks.
"""
import datetime as dt
import json
import subprocess
import sys
from pathlib import Path

import shutil

REPO = Path(__file__).resolve().parent.parent
S = Path(sys.argv[1]).resolve() / "sim"
BINARY = Path(sys.argv[2] if len(sys.argv) > 2 else REPO / "target/debug/akasha").resolve()
if S.exists():
    shutil.rmtree(S)
shutil.copytree(REPO / "tests/fixtures/resolution/valid-root", S / "root")
shutil.copytree(REPO / "tests/fixtures/resolution/repository", S / "repository")
for template in (REPO / "crates/akasha-core/defaults/templates").glob("*.md"):
    shutil.copy(template, S / "root/templates" / template.name)
AK = [str(BINARY), "--root", str(S / "root"), "--project", "example"]
P = S / "root/Projects/example"
SENT = ("Measured the change against the agreed acceptance criteria and recorded what remains open. "
        "The bounded path keeps sources exact while the fallback stays explicit and observable. ")


def prose(n):
    return (SENT * (n // len(SENT) + 1))[:n].rstrip() + "."


def ak(*args):
    r = subprocess.run(AK + list(args), capture_output=True, text=True)
    if r.returncode:
        sys.exit(f"command failed: {' '.join(args[:3])}: {r.stderr.strip()}")
    return r.stdout


def projection(name, new_text):
    tmp = S / f"{name}.next.md"
    tmp.write_text(new_text)
    return str(tmp)


def create_record(kind, slug, title, day, body):
    road = (P / "roadmap.md").read_text() + f"- [ ] [[records/{kind}s/{slug}|{title}]] — planned slice: {prose(90)}\n"
    ak("create-note", kind, f"{slug}.md", "--projection", projection("roadmap", road),
       "--field", "status=open", "--field", f"title={title}", "--field", f"created={day}",
       "--field", f"updated={day}", "--field", f"body={body}")


def update_record(kind, slug, status, day, append):
    path = P / f"records/{kind}s/{slug}.md"
    old = path.read_text()
    new = old.replace("status: open", f"status: {status}").replace("status: active", f"status: {status}")
    new = "\n".join(l if not l.startswith("updated:") else f"updated: {day}" for l in new.split("\n"))
    new = new.rstrip("\n") + "\n\n" + append + "\n"
    road = (P / "roadmap.md").read_text()
    if status == "done":
        road = road.replace(f"- [ ] [[records/{kind}s/{slug}|", f"- [x] [[records/{kind}s/{slug}|")
    (S / "expected.md").write_text(old)
    (S / "replacement.md").write_text(new)
    ak("update-record", f"Projects/example/records/{kind}s/{slug}.md", "--expected", str(S / "expected.md"),
       "--replacement", str(S / "replacement.md"), "--roadmap", projection("roadmap", road))


def snapshot(label):
    data = json.loads(ak("--json", "context"))
    kept, dropped = {}, {}
    for e in data["entries"]:
        kept[e["section"]] = kept.get(e["section"], 0) + 1
    for o in data["omissions"]:
        dropped[o["section"]] = dropped.get(o["section"], 0) + 1
    sizes = {n: len((P / n).read_text()) for n in ("roadmap.md", "index.md")}
    print(f"{label:9} rendered={data['rendered_chars']:6} kept={kept} omitted={dropped} sizes={sizes}")


start = dt.date(2026, 8, 3)
open_tasks, open_problems, n_task, n_prob, n_ent = [], [], 0, 0, 0
for week in range(1, 27):
    for d in range(3):
        day = (start + dt.timedelta(weeks=week, days=d)).isoformat()
        ak("create-event", "session", f"{day}-w{week}.md", "--field", f"date={day}",
           "--field", f"title=Session {day}", "--field", f"body={prose(1500)}")
    day = (start + dt.timedelta(weeks=week, days=4)).isoformat()
    for _ in range(2):
        n_task += 1
        slug = f"task-{n_task:03}"
        create_record("task", slug, f"Task {n_task}", day, prose(600))
        open_tasks.append(slug)
    for slug in open_tasks[:]:  # every open task gains a progress note each week
        update_record("task", slug, "active", day, f"## Progress {day}\n\n{prose(250)}")
    target_open = min(3 + week // 2, 14)
    while len(open_tasks) > target_open:
        update_record("task", open_tasks.pop(0), "done", day, f"## Outcome\n\n{prose(200)}")
    if week % 2 == 0:
        n_prob += 1
        create_record("problem", f"problem-{n_prob:03}", f"Problem {n_prob}", day, prose(1000))
        open_problems.append(f"problem-{n_prob:03}")
        if len(open_problems) > 5:
            update_record("problem", open_problems.pop(0), "resolved", day, prose(200))
    if week % 4 == 0:
        road = (P / "roadmap.md").read_text() + f"\n## Milestone {week // 4}\n\n{prose(600)}\n\n"
        # roadmap-only edit is accepted together with the next record change; append the line now
        (S / "pending_roadmap.md").write_text(road)
        n_task += 1
        slug = f"task-{n_task:03}"
        road += f"- [ ] [[records/tasks/{slug}|Task {n_task}]] — planned slice: {prose(90)}\n"
        ak("create-note", "task", f"{slug}.md", "--projection", projection("roadmap", road),
           "--field", "status=open", "--field", f"title=Task {n_task}", "--field", f"created={day}",
           "--field", f"updated={day}", "--field", f"body={prose(600)}")
        open_tasks.append(slug)
    if n_ent < 40:
        n_ent += 1
        index = (P / "index.md").read_text() + f"- [[entities/comp-{n_ent:02}|comp-{n_ent:02}]] — subsystem owning one bounded responsibility.\n"
        ak("create-note", "entity", f"comp-{n_ent:02}.md", "--projection", projection("index", index),
           "--field", f"entity=comp-{n_ent:02}", "--field", "kind=subsystem", "--field", "status=active",
           "--field", f"reviewed={day}", "--field", f"title=comp-{n_ent:02}", "--field", f"body={prose(800)}")
    ak("create-event", "handoff", f"{day}.md", "--field", f"date={day}", "--field", f"title=Handoff {day}",
       "--field", f"body={prose(2000)}")
    if week in (1, 4, 8, 13, 26):
        snapshot(f"week {week}")
