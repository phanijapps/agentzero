"""Reproducible expressiveness spike for RFC-0016's constrained role map."""

from pathlib import PurePosixPath
import re


KEBAB = re.compile(r"^[a-z0-9]+(?:-[a-z0-9]+)*$")
ROLES = {
    "root": "{conceptPath}",
    "index": "{conceptPath}/index.md",
    "document": "{conceptPath}/{conceptName}.md",
    "spec": "{conceptPath}/spec.md",
    "plan": "{conceptPath}/plan.md",
    "tasks.root": "{conceptPath}/tasks",
    "tasks.index": "{conceptPath}/tasks/index.md",
    "tasks.document": "{conceptPath}/tasks/{task}.md",
    "history.root": "{conceptPath}/history",
    "history.index": "{conceptPath}/history/index.md",
    "runs.root": "{conceptPath}/history/runs",
    "runs.index": "{conceptPath}/history/runs/index.md",
    "runs.document": "{conceptPath}/history/runs/{run}.md",
}
RESOURCES = {
    "source": "src",
    "source.index": "src/index.md",
    "data": "data",
    "data.index": "data/index.md",
    "reports": "reports",
    "reports.index": "reports/index.md",
    "output": "output",
    "output.index": "output/index.md",
}


def safe_path(value: str) -> str:
    path = PurePosixPath(value)
    assert not path.is_absolute(), value
    assert "." not in path.parts and ".." not in path.parts, value
    assert str(path) == value, value
    return value


def resolve(concept_path: str) -> dict[str, str]:
    components = PurePosixPath(concept_path).parts
    assert components and all(KEBAB.fullmatch(part) for part in components)
    values = {
        "conceptPath": concept_path,
        "conceptName": components[-1],
        "task": "refresh-data",
        "run": "2026-07-19-001",
    }
    result = {name: safe_path(pattern.format(**values)) for name, pattern in ROLES.items()}
    assert len(result) == len(set(result.values()))
    return result


def main() -> None:
    all_paths: set[str] = set()
    for concept in (
        "aapl-analysis",
        "great-expectations",
        "great-expectations/chapter-01",
    ):
        resolved = resolve(concept)
        assert all_paths.isdisjoint(resolved.values())
        all_paths.update(resolved.values())
        print(f"{concept}: {len(resolved)} unique roles")

    resources = {name: safe_path(path) for name, path in RESOURCES.items()}
    assert len(resources) == len(set(resources.values()))
    assert all_paths.isdisjoint(resources.values())

    # The catalog consumes each ward snapshot's exported ward.index role.
    ward_indexes = {"default-ward": "index.md", "custom-ward": "home.md"}
    catalog_targets = {
        ward: safe_path(f"{ward}/{index}") for ward, index in ward_indexes.items()
    }
    assert catalog_targets["custom-ward"] == "custom-ward/home.md"

    print(f"resources: {len(resources)} unique roles")
    print(f"catalog targets: {catalog_targets}")
    print("spike: PASS")


if __name__ == "__main__":
    main()
