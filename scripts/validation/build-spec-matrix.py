#!/usr/bin/env python3
import json
import re
from collections import Counter
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
ACTIVE_SPECS = ROOT / "openspec" / "changes" / "windows-proxy-mvp" / "specs"
SPECS = ACTIVE_SPECS if ACTIVE_SPECS.is_dir() else ROOT / "openspec" / "specs"
RESULTS = ROOT / "docs" / "validation" / "evidence" / "spec-matrix-results.json"
OUTPUT = ROOT / "docs" / "validation" / "windows-spec-matrix.md"


def read_scenarios(path):
    requirement = None
    scenario_requirement = None
    scenario = None
    when = None
    then = None
    rows = []
    for line in path.read_text(encoding="utf-8").splitlines():
        match = re.match(r"### Requirement: (.+)", line)
        if match:
            requirement = match.group(1)
            continue
        match = re.match(r"#### Scenario: (.+)", line)
        if match:
            if scenario is not None:
                rows.append((scenario_requirement, scenario, when, then))
            scenario = match.group(1)
            scenario_requirement = requirement
            when = None
            then = None
            continue
        if scenario is not None and line.startswith("- **WHEN** "):
            when = line.removeprefix("- **WHEN** ")
        if scenario is not None and line.startswith("- **THEN** "):
            then = line.removeprefix("- **THEN** ")
    if scenario is not None:
        rows.append((scenario_requirement, scenario, when, then))
    for row in rows:
        if any(value is None for value in row):
            raise ValueError(f"incomplete scenario in {path}: {row}")
    return rows


def cell(value):
    return value.replace("|", "\\|").replace("\n", "<br>")


data = json.loads(RESULTS.read_text(encoding="utf-8"))
platforms = [
    (
        "Windows 11",
        {
            "groups": data["groups"],
            "overrides": data["overrides"],
        },
    ),
    ("Windows 10", data["windows10"]),
]
lines = [
    "# Windows Specs 场景矩阵",
    "",
    "本文件由 `scripts/validation/build-spec-matrix.py` 从 OpenSpec 场景和机器可读结果生成。实测平台为 Windows 11 Pro 10.0.26200 x64 和 Windows 10 Pro 10.0.19045 x64；内核为 `1.14.1-socks-proxy.2`。",
    "",
    "状态含义：Pass 表示当前平台已有与场景范围一致的证据；Partial 表示只验证了部分行为；Pending 表示未执行。单元测试只作为辅助证据，不代替要求真实 Windows 行为的场景。",
    "",
]
seen = {
    platform_name: {"groups": set(), "overrides": set()}
    for platform_name, _ in platforms
}
scenario_count = 0
counts = {platform_name: Counter() for platform_name, _ in platforms}


def scenario_result(platform_name, platform_data, group_key, scenario_key):
    result = platform_data["groups"].get(group_key)
    if result is not None:
        seen[platform_name]["groups"].add(group_key)
    if scenario_key in platform_data["overrides"]:
        result = platform_data["overrides"][scenario_key]
        seen[platform_name]["overrides"].add(scenario_key)
    if result is None:
        return {
            "status": "Pending",
            "actual": "未执行。",
            "evidence": [],
        }
    return result


def render_result(result):
    evidence = "、".join(f"`{item}`" for item in result["evidence"])
    suffix = f" 证据：{evidence}" if evidence else ""
    return f"**{result['status']}**：{result['actual']}{suffix}"


for spec in sorted(SPECS.glob("*/spec.md")):
    capability = spec.parent.name
    lines.extend(
        [
            f"## {capability}",
            "",
            "| Scenario | 步骤 | 预期 | Windows 11 实际与证据 | Windows 10 实际与证据 |",
            "|---|---|---|---|---|",
        ]
    )
    for requirement, scenario, when, then in read_scenarios(spec):
        scenario_count += 1
        group_key = f"{capability}/{requirement}"
        scenario_key = f"{group_key}/{scenario}"
        rendered = []
        for platform_name, platform_data in platforms:
            result = scenario_result(
                platform_name, platform_data, group_key, scenario_key
            )
            if result["status"] not in {"Pass", "Partial", "Pending"}:
                raise ValueError(
                    f"invalid status for {platform_name}/{scenario_key}: "
                    f"{result['status']}"
                )
            counts[platform_name][result["status"]] += 1
            rendered.append(render_result(result))
        lines.append(
            "| "
            + " | ".join(
                cell(value)
                for value in (scenario, when, then, *rendered)
            )
            + " |"
        )
    lines.append("")

for platform_name, platform_data in platforms:
    unused_groups = set(platform_data["groups"]) - seen[platform_name]["groups"]
    unused_overrides = (
        set(platform_data["overrides"]) - seen[platform_name]["overrides"]
    )
    if unused_groups or unused_overrides:
        raise KeyError(
            f"unused {platform_name} result keys: "
            f"groups={sorted(unused_groups)}, "
            f"overrides={sorted(unused_overrides)}"
        )
lines.extend(
    [
        "## 汇总",
        "",
        f"矩阵共 {scenario_count} 个 Scenario。"
        f"Windows 11：Pass {counts['Windows 11']['Pass']}，"
        f"Partial {counts['Windows 11']['Partial']}，"
        f"Pending {counts['Windows 11']['Pending']}；"
        f"Windows 10：Pass {counts['Windows 10']['Pass']}，"
        f"Partial {counts['Windows 10']['Partial']}，"
        f"Pending {counts['Windows 10']['Pending']}。"
        "两个平台的 Partial/Pending 项均是当前验收缺口，完成后重新运行生成器并复核证据。",
        "",
    ]
)
OUTPUT.write_text("\n".join(lines), encoding="utf-8")
print(f"wrote {OUTPUT} with {scenario_count} scenarios")
