from __future__ import annotations

import json
import re
import sys
from pathlib import Path

from jsonschema import Draft202012Validator, FormatChecker
from referencing import Registry, Resource


ROOT = Path(__file__).resolve().parents[1]
DOCS = ROOT / "docs"

REQUIRED_FILES = [
    "README.md",
    "AGENTS.md",
    "CONTRIBUTING.md",
    "SECURITY.md",
    "CHANGELOG.md",
    "src/AGENTS.md",
    "src-tauri/AGENTS.md",
    "tests/AGENTS.md",
    "docs/INDEX.md",
    "docs/GLOSSARY.md",
    "docs/product/PRD.md",
    "docs/product/FRS.md",
    "docs/product/NFRS.md",
    "docs/product/AI-BEHAVIOR.md",
    "docs/product/UX-SPEC.md",
    "docs/architecture/ARCHITECTURE.md",
    "docs/architecture/RUNTIME-STATE-MACHINES.md",
    "docs/architecture/AI-ORCHESTRATION.md",
    "docs/architecture/DATA-MODEL.md",
    "docs/architecture/DEPENDENCY-POLICY.md",
    "docs/contracts/API-CONTRACT.md",
    "docs/contracts/PROVIDER-CONTRACTS.md",
    "docs/integrations/EXTERNAL-INTEGRATIONS.md",
    "docs/security/THREAT-MODEL.md",
    "docs/security/PRIVACY-DATA-LIFECYCLE.md",
    "docs/security/LEGAL-AND-LICENSING.md",
    "docs/testing/TEST-STRATEGY.md",
    "docs/testing/ACCEPTANCE-TESTS.md",
    "docs/testing/TRACEABILITY.md",
    "docs/planning/ROADMAP.md",
    "docs/planning/BACKLOG.md",
    "docs/planning/RISK-REGISTER.md",
    "docs/operations/DEVELOPMENT-GUIDE.md",
    "docs/operations/BUILD-RELEASE.md",
    "docs/operations/RUNBOOK.md",
]

ADR_FILES = [
    f"docs/architecture/adr/ADR-{index:04d}-{slug}.md"
    for index, slug in [
        (1, "tauri-react-rust"),
        (2, "local-and-system-media-sources"),
        (3, "byok-and-local-first-memory"),
        (4, "provider-abstractions"),
        (5, "metadata-and-weather-providers"),
        (6, "document-and-contract-driven-development"),
        (7, "milestone-prototype-delivery"),
    ]
]

SCHEMA_BASES = [
    "program-plan",
    "playback-state",
    "playback-event",
    "memory-record",
    "schedule-rule",
    "provider-error",
]

DEFINITION_SOURCES = {
    "FR": ["docs/product/FRS.md"],
    "NFR": ["docs/product/NFRS.md"],
    "UX": ["docs/product/UX-SPEC.md"],
    "ARCH": [
        "docs/architecture/ARCHITECTURE.md",
        "docs/architecture/RUNTIME-STATE-MACHINES.md",
        "docs/architecture/AI-ORCHESTRATION.md",
        "docs/architecture/DATA-MODEL.md",
        "docs/architecture/DEPENDENCY-POLICY.md",
    ],
    "ADR": ADR_FILES,
    "API": ["docs/contracts/API-CONTRACT.md", "docs/contracts/PROVIDER-CONTRACTS.md"],
    "EVT": ["docs/contracts/API-CONTRACT.md"],
    "ERR": ["docs/contracts/API-CONTRACT.md", "docs/contracts/PROVIDER-CONTRACTS.md"],
    "TEST": ["docs/testing/ACCEPTANCE-TESTS.md"],
    "TASK": ["docs/planning/BACKLOG.md"],
    "RISK": ["docs/planning/RISK-REGISTER.md"],
}

# Canonical identifiers always end in a numeric sequence. Requiring that suffix
# prevents document names such as API-CONTRACT and UX-SPEC from being mistaken
# for identifiers.
ID_RE = re.compile(
    r"\b(FR|NFR|UX|ARCH|ADR|API|EVT|ERR|TEST|TASK|RISK)-(?:[A-Z][A-Z0-9]*-)*\d{3,4}\b"
)
MARKDOWN_LINK_RE = re.compile(r"(?<!!)\[[^\]]+\]\(([^)]+)\)")


def load_text(relative: str) -> str:
    return (ROOT / relative).read_text(encoding="utf-8")


def definition_ids(relative: str, family: str) -> list[str]:
    definitions: list[str] = []
    pattern = re.compile(
        rf"^(?:#{{1,6}}\s+|\|\s*|`)"
        rf"(({family})-(?:[A-Z][A-Z0-9]*-)*\d{{3,4}})\b",
        re.MULTILINE,
    )
    for match in pattern.finditer(load_text(relative)):
        definitions.append(match.group(1))
    if family == "ADR":
        file_match = re.search(r"ADR-\d{4}", Path(relative).name)
        if file_match and file_match.group(0) not in definitions:
            definitions.append(file_match.group(0))
    return definitions


def markdown_cells(line: str) -> list[str]:
    return [cell.strip() for cell in line.strip().strip("|").split("|")]


def main() -> int:
    errors: list[str] = []

    for relative in REQUIRED_FILES + ADR_FILES:
        if not (ROOT / relative).is_file():
            errors.append(f"missing required file: {relative}")

    markdown_files = sorted(
        {*(ROOT.glob("*.md")), *DOCS.rglob("*.md"), *(ROOT / "src").glob("AGENTS.md"), *(ROOT / "src-tauri").glob("AGENTS.md"), *(ROOT / "tests").glob("AGENTS.md")}
    )
    metadata_fields = ["Status", "Owner", "Last Verified", "Source of Truth For", "Related Documents"]
    for path in markdown_files:
        text = path.read_text(encoding="utf-8")
        relative = path.relative_to(ROOT).as_posix()
        for field in metadata_fields:
            if not re.search(rf"^\|\s*{re.escape(field)}\s*\|", text, re.MULTILINE):
                errors.append(f"missing metadata '{field}': {relative}")
        status_match = re.search(r"^\|\s*Status\s*\|\s*([^|]+?)\s*\|", text, re.MULTILINE)
        if status_match and status_match.group(1).strip() not in {"Draft", "Approved", "Superseded"}:
            errors.append(f"invalid document status '{status_match.group(1).strip()}': {relative}")
        verified_match = re.search(r"^\|\s*Last Verified\s*\|\s*(\d{4}-\d{2}-\d{2})\s*\|", text, re.MULTILINE)
        if not verified_match:
            errors.append(f"invalid Last Verified date: {relative}")
        if re.search(r"(?i)(?:\[(?:TODO|TBD)\]|\b(?:TODO|TBD)\s*:)", text):
            errors.append(f"unresolved marker: {relative}")

        for target in MARKDOWN_LINK_RE.findall(text):
            target = target.strip().strip("<>")
            if target.startswith(("http://", "https://", "mailto:", "#")):
                continue
            target_path = target.split("#", 1)[0]
            if not target_path or any(char in target_path for char in "{}*"):
                continue
            resolved = (path.parent / target_path).resolve()
            if not resolved.exists():
                errors.append(f"broken local link in {relative}: {target}")

    definitions: dict[str, str] = {}
    for family, sources in DEFINITION_SOURCES.items():
        for relative in sources:
            if not (ROOT / relative).is_file():
                continue
            for identifier in definition_ids(relative, family):
                previous = definitions.get(identifier)
                if previous:
                    errors.append(f"duplicate definition {identifier}: {previous}, {relative}")
                definitions[identifier] = relative

    for path in markdown_files:
        relative = path.relative_to(ROOT).as_posix()
        for match in ID_RE.finditer(path.read_text(encoding="utf-8")):
            identifier = match.group(0)
            if identifier not in definitions:
                errors.append(f"undefined ID {identifier} referenced by {relative}")

    traceability_path = ROOT / "docs/testing/TRACEABILITY.md"
    trace_test_edges: set[tuple[str, str]] = set()
    trace_task_edges: set[tuple[str, str]] = set()
    if traceability_path.is_file():
        traceability = traceability_path.read_text(encoding="utf-8")
        for identifier, source in definitions.items():
            if identifier.startswith(("FR-", "NFR-")) and identifier not in traceability:
                errors.append(f"untraced requirement {identifier} from {source}")
        for line in traceability.splitlines():
            if not re.match(r"^\|\s*(?:FR|NFR)-", line):
                continue
            cells = markdown_cells(line)
            if len(cells) < 7:
                errors.append(f"malformed traceability row: {line.strip()}")
                continue
            requirement_match = ID_RE.search(cells[0])
            if not requirement_match:
                errors.append(f"traceability row without requirement: {line.strip()}")
                continue
            requirement = requirement_match.group(0)
            if not re.search(r"\bUX-(?:[A-Z][A-Z0-9]*-)*\d{3,4}\b", cells[1]):
                errors.append(f"traceability row without UX ID: {requirement}")
            if not re.search(r"\bARCH-(?:[A-Z][A-Z0-9]*-)*\d{3,4}\b", cells[2]):
                errors.append(f"traceability row without ARCH ID: {requirement}")
            contract_is_precise = bool(
                re.search(r"\b(?:API|EVT|ERR)-(?:[A-Z][A-Z0-9]*-)*\d{3,4}\b", cells[3])
                or ".schema.json" in cells[3]
                or re.search(r"\]\(\.\./contracts/(?:API-CONTRACT|PROVIDER-CONTRACTS)\.md", cells[3])
            )
            if not contract_is_precise:
                errors.append(f"traceability row without precise contract reference: {requirement}")
            for test_id in re.findall(r"\bTEST-(?:[A-Z][A-Z0-9]*-)*\d{3,4}\b", cells[4]):
                trace_test_edges.add((requirement, test_id))
            for task_id in re.findall(r"\bTASK-(?:[A-Z][A-Z0-9]*-)*\d{3,4}\b", cells[5]):
                trace_task_edges.add((requirement, task_id))

    backlog_path = ROOT / "docs/planning/BACKLOG.md"
    backlog_edges: set[tuple[str, str]] = set()
    if backlog_path.is_file():
        for line in backlog_path.read_text(encoding="utf-8").splitlines():
            if not re.match(r"^\|\s*TASK-", line):
                continue
            cells = markdown_cells(line)
            task_match = ID_RE.search(cells[0]) if cells else None
            requirement_ids = (
                re.findall(r"\b(?:FR|NFR)-(?:[A-Z][A-Z0-9]*-)*\d{3,4}\b", cells[2])
                if len(cells) >= 3
                else []
            )
            if not task_match or not requirement_ids:
                errors.append(f"task without requirement mapping: {line.strip()}")
                continue
            for requirement in requirement_ids:
                backlog_edges.add((requirement, task_match.group(0)))

    acceptance_path = ROOT / "docs/testing/ACCEPTANCE-TESTS.md"
    acceptance_edges: set[tuple[str, str]] = set()
    if acceptance_path.is_file():
        for line in acceptance_path.read_text(encoding="utf-8").splitlines():
            if not re.match(r"^\|\s*TEST-", line):
                continue
            cells = markdown_cells(line)
            test_match = ID_RE.search(cells[0]) if cells else None
            requirement_ids = (
                re.findall(r"\b(?:FR|NFR)-(?:[A-Z][A-Z0-9]*-)*\d{3,4}\b", cells[1])
                if len(cells) >= 2
                else []
            )
            if not test_match or not requirement_ids:
                errors.append(f"test without requirement mapping: {line.strip()}")
                continue
            for requirement in requirement_ids:
                acceptance_edges.add((requirement, test_match.group(0)))

    for edge in sorted(acceptance_edges - trace_test_edges):
        errors.append(f"acceptance edge missing from traceability: {edge[0]} -> {edge[1]}")
    for edge in sorted(trace_test_edges - acceptance_edges):
        errors.append(f"traceability test edge missing from acceptance Covers: {edge[0]} -> {edge[1]}")
    for edge in sorted(backlog_edges - trace_task_edges):
        errors.append(f"backlog edge missing from traceability: {edge[0]} -> {edge[1]}")
    for edge in sorted(trace_task_edges - backlog_edges):
        errors.append(f"traceability task edge missing from backlog Requirements: {edge[0]} -> {edge[1]}")

    schema_directory = ROOT / "docs/contracts/schemas"
    example_directory = ROOT / "docs/contracts/examples"
    schemas: dict[str, dict] = {}
    registry = Registry()
    for base in SCHEMA_BASES:
        schema_path = schema_directory / f"{base}.schema.json"
        if not schema_path.is_file():
            errors.append(f"missing schema: {schema_path.relative_to(ROOT)}")
            continue
        try:
            schema = json.loads(schema_path.read_text(encoding="utf-8"))
            Draft202012Validator.check_schema(schema)
            schema_id = schema.get("$id")
            if not isinstance(schema_id, str):
                raise ValueError("schema must declare a string $id")
            schemas[base] = schema
            registry = registry.with_resource(schema_id, Resource.from_contents(schema))
        except Exception as exc:  # validation error must remain visible
            errors.append(f"invalid schema {schema_path.relative_to(ROOT)}: {exc}")

    for base, schema in schemas.items():
        validator = Draft202012Validator(schema, registry=registry, format_checker=FormatChecker())

        for kind in ("valid", "boundary", "invalid"):
            example_path = example_directory / f"{base}.{kind}.json"
            if not example_path.is_file():
                errors.append(f"missing {kind} example: {example_path.relative_to(ROOT)}")
                continue
            try:
                instance = json.loads(example_path.read_text(encoding="utf-8"))
            except Exception as exc:
                errors.append(f"invalid JSON {example_path.relative_to(ROOT)}: {exc}")
                continue
            validation_errors = list(validator.iter_errors(instance))
            if kind == "invalid" and not validation_errors:
                errors.append(f"invalid example unexpectedly passes: {example_path.relative_to(ROOT)}")
            if kind != "invalid" and validation_errors:
                errors.append(
                    f"{kind} example fails {example_path.relative_to(ROOT)}: {validation_errors[0].message}"
                )

    agents_files = [ROOT / "AGENTS.md", ROOT / "src/AGENTS.md", ROOT / "src-tauri/AGENTS.md", ROOT / "tests/AGENTS.md"]
    agents_bytes = sum(path.stat().st_size for path in agents_files if path.is_file())
    if agents_bytes >= 32 * 1024:
        errors.append(f"combined AGENTS size {agents_bytes} exceeds default 32 KiB budget")

    expected_agent_chains = {
        ".": [ROOT / "AGENTS.md"],
        "src": [ROOT / "AGENTS.md", ROOT / "src/AGENTS.md"],
        "src-tauri": [ROOT / "AGENTS.md", ROOT / "src-tauri/AGENTS.md"],
        "tests": [ROOT / "AGENTS.md", ROOT / "tests/AGENTS.md"],
    }
    for target, chain in expected_agent_chains.items():
        missing = [path.relative_to(ROOT).as_posix() for path in chain if not path.is_file()]
        if missing:
            errors.append(f"AGENTS load chain for {target} is incomplete: {', '.join(missing)}")
        if len({path.resolve() for path in chain}) != len(chain):
            errors.append(f"AGENTS load chain for {target} contains duplicate levels")

    if errors:
        print("Documentation verification failed:")
        for error in errors:
            print(f"- {error}")
        return 1

    print(
        f"Documentation verification passed: {len(markdown_files)} Markdown files, "
        f"{len(definitions)} defined IDs, {len(SCHEMA_BASES)} schemas, "
        f"{len(acceptance_edges)} requirement-test edges, {len(backlog_edges)} requirement-task edges, "
        f"{len(expected_agent_chains)} AGENTS load chains, AGENTS {agents_bytes} bytes."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
