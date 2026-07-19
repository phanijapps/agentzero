#!/usr/bin/env python3
"""Verify Rig stays confined to the reviewed runtime adapter boundary."""

from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
RIG_DEP_NAMES = {"rig", "rig-core", "rig_core"}
ALLOWED_RIG_PACKAGE = "agent-runtime"
SOURCE_DIRS = ["apps", "discovery", "framework", "gateway", "runtime", "services", "stores"]
ALLOWED_RIG_SOURCE_FILE = Path("runtime/agent-runtime/src/rig_adapter.rs")
ALLOWED_RIG_SOURCE_DIR = Path("runtime/agent-runtime/src/rig_adapter")
RIG_IMPORT_RE = re.compile(
    r"\b(?:use|extern\s+crate)\s+(?:::)?(?:rig|rig_core)\b"
    r"|\b(?:rig|rig_core)\s*::"
    r"|\brig_core\b"
)
RIG_PROVIDER_DIRECT_RE = re.compile(r"\b(?:rig|rig_core)\s*::\s*providers\b")
RIG_PROVIDER_GROUP_RE = re.compile(
    r"\buse\s+(?:::)?(?:rig|rig_core)\s*::\s*\{[^;]*\bproviders\b",
    re.DOTALL,
)
RIG_ALIAS_RE = re.compile(
    r"\buse\s+(?:::)?(?:rig|rig_core)\s+as\s+([A-Za-z_][A-Za-z0-9_]*)\s*;"
    r"|\bextern\s+crate\s+(?:rig|rig_core)\s+as\s+([A-Za-z_][A-Za-z0-9_]*)\s*;"
    r"|\buse\s+(?:::)?(?:rig|rig_core)\s*::\s*\{[^;]*\bself\s+as\s+([A-Za-z_][A-Za-z0-9_]*)\b",
    re.DOTALL,
)


def cargo_metadata() -> dict:
    result = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--no-deps"],
        cwd=REPO_ROOT,
        check=True,
        text=True,
        stdout=subprocess.PIPE,
    )
    return json.loads(result.stdout)


def fail(message: str) -> None:
    print(f"rig_boundary_check: {message}", file=sys.stderr)
    raise SystemExit(1)


def check_direct_dependencies(metadata: dict) -> None:
    violations: list[str] = []
    for package in metadata["packages"]:
        package_name = package["name"]
        for dep in package["dependencies"]:
            dep_name = dep["name"]
            if dep_name in RIG_DEP_NAMES and package_name != ALLOWED_RIG_PACKAGE:
                violations.append(f"{package_name} -> {dep_name}")
            if dep_name == "reqwest" and dep.get("req", "").startswith("^0.13"):
                violations.append(f"{package_name} directly requests reqwest {dep['req']}")

    if violations:
        fail("dependency boundary violations: " + ", ".join(sorted(violations)))


def uses_rig_provider(text: str) -> bool:
    if RIG_PROVIDER_DIRECT_RE.search(text) or RIG_PROVIDER_GROUP_RE.search(text):
        return True

    aliases = {
        alias
        for match in RIG_ALIAS_RE.finditer(text)
        for alias in match.groups()
        if alias is not None
    }
    return any(
        re.search(rf"\b{re.escape(alias)}\s*::\s*providers\b", text)
        or re.search(rf"\buse\s+(?:::)?{re.escape(alias)}\s*::\s*\{{[^;]*\bproviders\b", text, re.DOTALL)
        for alias in aliases
    )


def is_allowed_rig_source(relative: Path) -> bool:
    return relative == ALLOWED_RIG_SOURCE_FILE or relative.is_relative_to(ALLOWED_RIG_SOURCE_DIR)


def check_source_imports() -> None:
    violations: list[str] = []
    for root_name in SOURCE_DIRS:
        root = REPO_ROOT / root_name
        if not root.exists():
            continue
        for path in root.rglob("*.rs"):
            relative = path.relative_to(REPO_ROOT)
            text = path.read_text(encoding="utf-8")

            if uses_rig_provider(text):
                violations.append(f"{relative}: Rig-native provider import is forbidden")
                continue

            if is_allowed_rig_source(relative):
                continue

            if RIG_IMPORT_RE.search(text):
                violations.append(f"{relative}: Rig import outside runtime adapter boundary")

    if violations:
        fail("source boundary violations: " + "; ".join(violations))


def main() -> int:
    metadata = cargo_metadata()
    check_direct_dependencies(metadata)
    check_source_imports()
    print("rig-boundary-clean")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
