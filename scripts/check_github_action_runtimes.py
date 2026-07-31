#!/usr/bin/env python3
"""Fail when repository workflows select superseded JavaScript action majors."""

from __future__ import annotations

import re
import sys
from pathlib import Path

WORKSPACE = Path(__file__).resolve().parents[1]
WORKFLOW_DIR = WORKSPACE / ".github" / "workflows"
REQUIRED_MAJORS = {
    "sonarsource/sonarqube-scan-action": "v8",
    "actions/checkout": "v7",
    "actions/download-artifact": "v8",
    "actions/setup-node": "v7",
    "actions/setup-python": "v7",
    "actions/upload-artifact": "v7",
    "codecov/codecov-action": "v7",
    "gitleaks/gitleaks-action": "v3",
    "softprops/action-gh-release": "v3",
}
USES_PATTERN = re.compile(
    r"""^\s*(?:-\s*)?uses:\s*
        (?P<quote>["']?)
        (?P<action>[^@"'\s]+)@(?P<ref>[^#"'\s]+)
        (?P=quote)\s*(?:\#.*)?$
    """,
    re.VERBOSE,
)


def parse_uses(line: str) -> tuple[str, str] | None:
    """Return an action and ref from a complete YAML ``uses`` scalar."""
    match = USES_PATTERN.match(line)
    if match is None:
        return None
    return match.group("action"), match.group("ref")


def validate_parser() -> None:
    """Keep quoted and unquoted workflow syntax inside the policy boundary."""
    expected = ("actions/checkout", "v7")
    assert parse_uses("- uses: actions/checkout@v7") == expected
    assert parse_uses('  uses: "actions/checkout@v7"') == expected
    assert parse_uses("  uses: 'actions/checkout@v7' # pinned major") == expected
    assert parse_uses('  uses: "actions/checkout@v7') is None
    assert parse_uses("- uses: Actions/Checkout@v7") == ("Actions/Checkout", "v7")


def main() -> int:
    validate_parser()
    failures: list[str] = []
    seen: set[str] = set()

    workflows = sorted((*WORKFLOW_DIR.glob("*.yml"), *WORKFLOW_DIR.glob("*.yaml")))
    for workflow in workflows:
        for line_number, line in enumerate(
            workflow.read_text(encoding="utf-8").splitlines(), start=1
        ):
            parsed = parse_uses(line)
            if parsed is None:
                normalized_line = line.lower()
                if any(action in normalized_line for action in REQUIRED_MAJORS):
                    failures.append(
                        f"{workflow.relative_to(WORKSPACE)}:{line_number}: "
                        "could not parse governed action reference"
                    )
                continue
            action, selected_ref = parsed
            policy_action = action.lower()
            required_ref = REQUIRED_MAJORS.get(policy_action)
            if required_ref is None:
                continue
            seen.add(policy_action)
            if selected_ref != required_ref:
                failures.append(
                    f"{workflow.relative_to(WORKSPACE)}:{line_number}: "
                    f"{action}@{selected_ref} must select @{required_ref}"
                )

    for missing in sorted(REQUIRED_MAJORS.keys() - seen):
        failures.append(f"expected action family is not present in workflows: {missing}")

    if failures:
        print("GitHub Action runtime policy violations:", file=sys.stderr)
        for failure in failures:
            print(f"- {failure}", file=sys.stderr)
        return 1

    print(
        f"github-action-runtimes-clean: {len(seen)} action families across "
        f"{len(workflows)} workflows"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
