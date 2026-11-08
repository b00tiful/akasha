#!/usr/bin/env python3
"""Measure agent token usage for this repository from local Codex rollouts.

Usage: scripts/codex-usage.py [--since YYYY-MM-DD]

Reports per-session tokens, how many model turns followed documentation versus code tool calls,
and how much tool output came from workbench notes. Use it to compare workflow changes.
"""
import collections
import glob
import json
import os
import re
import sys

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
since = sys.argv[sys.argv.index("--since") + 1] if "--since" in sys.argv else "0000-00-00"
DOC = re.compile(r"akasha/[A-Z_0-9]|VAULT\.md|AGENTS\.md|Hub\.md")
CODE = re.compile(r"crates/|apps/|\bcargo\b|\bnode\b|\bnpm\b|codegraph_explore|codegraph explore|scripts/|tests/")


def kind(cmd):
    patched = re.findall(r"\*\*\* (?:Update|Add|Delete) File: (\S+)", cmd)
    if patched:
        doc = all("/akasha/akasha/" in p or p.endswith(("VAULT.md", "AGENTS.md")) for p in patched)
        return "doc-write" if doc else "code-write"
    if DOC.search(cmd) and not CODE.search(cmd):
        return "doc-read"
    return "code/test" if CODE.search(cmd) else "other"


rows, by_kind, out_kind = [], collections.Counter(), collections.Counter()
for path in sorted(glob.glob(os.path.expanduser("~/.codex/sessions/*/*/*/*.jsonl"))):
    stamp = os.path.basename(path)[8:27]
    if stamp[:10] < since:
        continue
    meta, total, turns, peak, pending, calls = {}, None, 0, 0, "start", {}
    for line in open(path, encoding="utf-8"):
        try:
            rec = json.loads(line)
        except ValueError:
            continue
        p = rec.get("payload") or {}
        if rec.get("type") == "session_meta":
            meta = p
        if not isinstance(p, dict):
            continue
        t = p.get("type")
        if t in ("custom_tool_call", "function_call"):
            cmd = p.get("input") or p.get("arguments") or ""
            pending = calls[p.get("call_id")] = kind(cmd)
        elif t in ("custom_tool_call_output", "function_call_output"):
            out = p.get("output")
            out_kind[calls.get(p.get("call_id"), "other")] += len(out if isinstance(out, str) else json.dumps(out))
        elif t == "token_count" and p.get("info"):
            total = p["info"]["total_token_usage"]
            last = p["info"]["last_token_usage"]["input_tokens"]
            turns += 1
            peak = max(peak, last)
            by_kind[pending] += last
    if total and meta.get("cwd", "").startswith(REPO):
        rows.append((stamp, turns, total["input_tokens"], total["cached_input_tokens"],
                     total["output_tokens"], total.get("reasoning_output_tokens", 0), peak))

print(f"{'session':19} {'turns':>5} {'input':>12} {'cached':>12} {'output':>9} {'reasoning':>9} {'peak ctx':>8}")
for r in rows:
    print(f"{r[0]:19} {r[1]:5} {r[2]:12,} {r[3]:12,} {r[4]:9,} {r[5]:9,} {r[6]:8,}")
if not rows:
    sys.exit("codex-usage: no rollouts for this repository in the selected range")
inp, out, turns = sum(r[2] for r in rows), sum(r[4] for r in rows), sum(r[1] for r in rows)
print(f"\n{len(rows)} sessions, {turns} turns, input {inp:,} (mean context {inp // max(turns, 1):,}), output {out:,}")
T, O = sum(by_kind.values()) or 1, sum(out_kind.values()) or 1
print("turn input after each tool kind: " + ", ".join(f"{k} {100 * v / T:.0f}%" for k, v in by_kind.most_common()))
print("tool output characters by kind:  " + ", ".join(f"{k} {100 * v / O:.0f}%" for k, v in out_kind.most_common()))
