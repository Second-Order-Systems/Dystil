#!/usr/bin/env python3
"""Verify that citations in agent_docs/ still resolve against the code.

Only files with `status: verified` frontmatter are checked. `unreviewed` and
`narrative` documents are skipped by design.

Two citation forms are understood:

    `path/to/file.rs :: symbol_name`     -> file must exist and contain the symbol
    `path/to/file.rs`                    -> file must exist

`path` may be a bare basename (`app_config.rs`) when it is unambiguous in the
repo, or a repo-relative path.

Deliberately does NOT understand `path:line` — line numbers drift on unrelated
edits, and a check that false-fires gets disabled. See AGENTS.md.

Exit code 1 on any unresolved citation.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
DOCS = REPO / "agent_docs"

SKIP_DIRS = {"target", "node_modules", ".git", "out", "dist", ".next"}
CODE_EXT = {".rs", ".ts", ".tsx", ".js", ".jsx", ".toml", ".json", ".yml", ".yaml", ".sh", ".md"}

# `foo.rs :: symbol` or `path/foo.rs :: symbol`
CITE_SYMBOL = re.compile(r"`([\w./-]+\.\w+)\s*::\s*([\w:<>]+)\(?\)?`")
# a backticked repo-relative path containing a slash
CITE_PATH = re.compile(r"`((?:[\w.-]+/)+[\w.-]+\.\w+)`")


def index_repo() -> dict[str, list[Path]]:
    """Map basename -> list of matching files, skipping build output."""
    by_name: dict[str, list[Path]] = {}
    for p in REPO.rglob("*"):
        if not p.is_file() or p.suffix not in CODE_EXT:
            continue
        if any(part in SKIP_DIRS for part in p.parts):
            continue
        by_name.setdefault(p.name, []).append(p)
    return by_name


def resolve(ref: str, by_name: dict[str, list[Path]]) -> list[Path]:
    """Resolve a citation reference to candidate files."""
    direct = REPO / ref
    if direct.is_file():
        return [direct]
    # try suffix match on repo-relative paths, else basename
    if "/" in ref:
        return [p for p in by_name.get(Path(ref).name, []) if str(p).endswith(ref)]
    return by_name.get(ref, [])


def main() -> int:
    if not DOCS.is_dir():
        print(f"no {DOCS.relative_to(REPO)}/ directory; nothing to check")
        return 0

    by_name = index_repo()
    failures: list[str] = []
    checked = 0

    valid_status = {"verified", "unreviewed", "narrative"}

    for doc in sorted(DOCS.glob("*.md")):
        text = doc.read_text(encoding="utf-8")
        rel = doc.relative_to(REPO)

        # Every agent_docs file must declare a status. An unmarked file reads as
        # authoritative while being unchecked — that is exactly how the old
        # TECHNICAL_ARCHITECTURE.md fiction survived. See AGENTS.md.
        m = re.search(r"^status:\s*(\S+)\s*$", text, re.MULTILINE)
        if not m:
            failures.append(
                f"{rel}: missing `status:` frontmatter "
                f"(one of: {', '.join(sorted(valid_status))})"
            )
            continue
        if m.group(1) not in valid_status:
            failures.append(
                f"{rel}: unknown status `{m.group(1)}` "
                f"(expected one of: {', '.join(sorted(valid_status))})"
            )
            continue
        if m.group(1) != "verified":
            continue

        for ref, symbol in CITE_SYMBOL.findall(text):
            checked += 1
            matches = resolve(ref, by_name)
            if not matches:
                failures.append(f"{rel}: no such file `{ref}` (cited with :: {symbol})")
                continue
            if not any(symbol in m.read_text(encoding="utf-8", errors="ignore") for m in matches):
                where = matches[0].relative_to(REPO) if len(matches) == 1 else f"{len(matches)} candidates"
                failures.append(f"{rel}: `{symbol}` not found in {where} — code moved or was renamed")

        for ref in CITE_PATH.findall(text):
            # documents reference each other; those are not code citations
            if ref.startswith(("agent_docs/", "public_docs/")):
                continue
            checked += 1
            if not resolve(ref, by_name):
                failures.append(f"{rel}: no such file `{ref}`")

    if failures:
        print("Documentation citations no longer resolve:\n")
        for f in failures:
            print(f"  {f}")
        print(
            f"\n{len(failures)} broken of {checked} checked.\n"
            "Fix the document (or the citation) in this change — see AGENTS.md."
        )
        return 1

    print(f"agent_docs citations OK ({checked} checked)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
