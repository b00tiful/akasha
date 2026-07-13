#!/usr/bin/env python3
"""Small REST client for the temporary Akasha knowledge workbench."""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
from datetime import date, datetime
from pathlib import Path
from typing import Any
from urllib.error import HTTPError, URLError
from urllib.parse import quote, urlencode
from urllib.request import Request, urlopen


PROJECT_ROOT = Path(__file__).resolve().parent.parent
ENV_FILE = PROJECT_ROOT / ".env"


def load_env(path: Path) -> None:
    """Load simple KEY=VALUE entries without executing the file as shell code."""
    if not path.is_file():
        raise RuntimeError(f"Missing {path}; copy .env.example and set OBSIDIAN_API_KEY.")

    for raw_line in path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        key = key.strip()
        value = value.strip()
        if len(value) >= 2 and value[0] == value[-1] and value[0] in {"'", '"'}:
            value = value[1:-1]
        os.environ.setdefault(key, value)


load_env(ENV_FILE)

API_KEY = os.environ.get("OBSIDIAN_API_KEY", "")
API_BASE = os.environ.get(
    "OBSIDIAN_BASE_URL",
    f"http://127.0.0.1:{os.environ.get('OBSIDIAN_PORT', '27123')}",
).rstrip("/")

if not API_KEY:
    raise RuntimeError(f"OBSIDIAN_API_KEY is empty in {ENV_FILE}.")


def request(
    method: str,
    endpoint: str,
    *,
    body: bytes | None = None,
    content_type: str | None = None,
) -> bytes:
    """Perform one authenticated Local REST API request."""
    headers = {
        "Authorization": f"Bearer {API_KEY}",
        "Accept": "application/json, text/markdown, */*",
    }
    if content_type is not None:
        headers["Content-Type"] = content_type

    req = Request(
        f"{API_BASE}{endpoint}",
        data=body,
        headers=headers,
        method=method,
    )
    try:
        with urlopen(req, timeout=15) as response:
            return response.read()
    except HTTPError as exc:
        detail = exc.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"Obsidian REST {method} {endpoint} failed: HTTP {exc.code}: {detail}") from exc
    except URLError as exc:
        raise RuntimeError(
            f"Cannot reach Obsidian at {API_BASE}. Start Obsidian and enable Local REST API."
        ) from exc


def vault_endpoint(path: str) -> str:
    """Return a safely encoded REST endpoint for a vault-relative path."""
    normalized = path.strip().lstrip("/")
    return f"/vault/{quote(normalized, safe='/')}"


def read_note(path: str) -> str:
    """Read one Markdown note."""
    return request("GET", vault_endpoint(path)).decode("utf-8")


def write_note(path: str, content: str) -> None:
    """Create or replace one Markdown note."""
    request(
        "PUT",
        vault_endpoint(path),
        body=content.encode("utf-8"),
        content_type="text/markdown; charset=utf-8",
    )


def list_path(path: str) -> list[str]:
    """List files below one vault-relative folder."""
    payload = json.loads(request("GET", vault_endpoint(path)).decode("utf-8"))
    files = payload.get("files", []) if isinstance(payload, dict) else []
    return [str(item) for item in files]


def search_notes(query: str) -> list[dict[str, Any]]:
    """Run Local REST API simple search."""
    encoded = urlencode({"query": query})
    payload = request(
        "POST",
        f"/search/simple/?{encoded}",
        body=b"",
        content_type="application/json",
    )
    parsed = json.loads(payload.decode("utf-8"))
    return parsed if isinstance(parsed, list) else []


def append_note(path: str, content: str) -> None:
    """Append using a read/replace cycle supported by every plugin version."""
    try:
        existing = read_note(path)
    except RuntimeError as exc:
        if "HTTP 404" not in str(exc):
            raise
        existing = ""
    separator = "" if not existing or existing.endswith("\n") else "\n"
    write_note(path, existing + separator + content)


def context_bundle() -> str:
    """Return the bounded startup context used by the legacy workflow."""
    sections = [
        ("Project index", read_note("00_INDEX.md")),
        ("Active tasks", read_note("Tasks/active.md")),
    ]
    session_files = sorted(
        item for item in list_path("Sessions/") if item.lower().endswith(".md")
    )[-2:]
    for session in session_files:
        session_path = session if session.startswith("Sessions/") else f"Sessions/{session}"
        sections.append((f"Session: {Path(session_path).name}", read_note(session_path)))
    return "\n\n".join(f"## {title}\n\n{content.strip()}" for title, content in sections)


def log_action(args: argparse.Namespace) -> None:
    """Append a correctly shaped action to today's session note."""
    today = date.today().isoformat()
    session_path = f"Sessions/{today}.md"
    try:
        read_note(session_path)
    except RuntimeError as exc:
        if "HTTP 404" not in str(exc):
            raise
        write_note(
            session_path,
            (
                "---\n"
                "tags: [session]\n"
                f"date: {today}\n"
                "status: active\n"
                "topics: []\n"
                "---\n\n"
                f"# Session: {today}\n\n"
                "## Goal\n\nContinue current project work.\n\n"
                "## Log\n\n"
                "## Related\n\n"
                "- [[Tasks/active|Active tasks]]\n"
                "- [[00_INDEX|Project index]]\n"
            ),
        )

    files = args.files.strip() or "none"
    entry = (
        f"\n## {datetime.now().strftime('%H:%M')} — {args.title}\n"
        f"Task: {args.task or 'not specified'}\n"
        f"Done: {args.done or 'not specified'}\n"
        f"Files changed: {files}\n"
        f"Result: {args.result}\n"
        f"Problems: {args.problems or 'none'}\n"
        f"Next: {args.next_step or 'not specified'}\n"
    )
    append_note(session_path, entry)
    print(f"LOG_OK {session_path}")


def create_problem(args: argparse.Namespace) -> None:
    """Create a template-compatible problem note linked into the graph."""
    slug = re.sub(r"[^a-z0-9]+", "-", args.title.lower()).strip("-")[:80]
    if not slug:
        raise RuntimeError("Problem title must contain letters or numbers.")
    today = date.today().isoformat()
    path = f"Problems/{slug}.md"
    content = f"""---
tags: [problem]
status: open
severity: {args.severity}
created: {today}
resolved:
---

# Problem: {args.title}

## Description

{args.description or 'Not specified.'}

## Expected behavior

Not specified.

## Steps to reproduce

1. Not specified.

## Evidence

{args.evidence or 'Not specified.'}

## Attempted solutions

| Attempt | What was tried | Result |
|---|---|---|
| 1 | Not attempted | Pending |

## Resolution

Open.

## Related

- [[Tasks/active|Active tasks]]
- [[Sessions/{today}|Discovery session]]
"""
    write_note(path, content)
    print(f"PROBLEM_OK {path}")


def build_parser() -> argparse.ArgumentParser:
    """Build the command-line interface."""
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    sub.add_parser("health", help="Verify REST authentication and index access.")
    sub.add_parser("context", help="Print index, active tasks, and the latest two sessions.")

    read = sub.add_parser("read", help="Read one vault-relative note.")
    read.add_argument("path")

    search = sub.add_parser("search", help="Search note contents.")
    search.add_argument("query")

    log = sub.add_parser("log", help="Append one action to today's session.")
    log.add_argument("title")
    log.add_argument("--task", default="")
    log.add_argument("--done", default="")
    log.add_argument("--files", default="")
    log.add_argument("--result", choices=("success", "partial", "failed"), default="success")
    log.add_argument("--problems", default="")
    log.add_argument("--next", dest="next_step", default="")

    problem = sub.add_parser("problem", help="Create a graph-linked problem note.")
    problem.add_argument("title")
    problem.add_argument("--severity", choices=("blocker", "major", "minor"), default="major")
    problem.add_argument("--description", default="")
    problem.add_argument("--evidence", default="")
    return parser


def main() -> int:
    """Run the requested operation with concise failures."""
    args = build_parser().parse_args()
    try:
        if args.command == "health":
            index = read_note("00_INDEX.md")
            if "# Akasha — Project Index" not in index:
                raise RuntimeError("00_INDEX.md did not contain the expected project heading.")
            print("HEALTH_OK")
        elif args.command == "context":
            print(context_bundle())
        elif args.command == "read":
            print(read_note(args.path))
        elif args.command == "search":
            results = search_notes(args.query)
            for item in results[:10]:
                filename = item.get("filename") or item.get("path") or "unknown"
                score = item.get("score", "n/a")
                print(f"{filename}\tscore={score}")
            if not results:
                print("NO_RESULTS")
        elif args.command == "log":
            log_action(args)
        elif args.command == "problem":
            create_problem(args)
    except (RuntimeError, json.JSONDecodeError) as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

