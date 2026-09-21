#!/usr/bin/env python3
"""Collect licenses for modules linked into the pinned Windows build."""

import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import urllib.request
import zipfile


BASE = Path(__file__).resolve().parent
REPO = BASE.parents[1]
LOCK = json.loads((BASE / "source-lock.json").read_text())
SOURCE = REPO / ".work" / ("sing-box-" + LOCK["commit"])
OUTPUT = REPO / "dist" / "sing-box" / "licenses"
LICENSE_NAMES = ("LICENSE", "LICENSE.txt", "LICENSE.md", "COPYING", "NOTICE", "PATENTS")


def classify(text: str) -> list[str]:
    lower = text.lower()
    beginning = lower[:1500]
    if "prebuilt binaries license" in beginning:
        return ["Wintun-Prebuilt"]
    if "mozilla public license" in beginning:
        return ["MPL-2.0"]
    if "apache license" in beginning and "version 2.0" in beginning:
        return ["Apache-2.0"]
    if "gnu general public license" in beginning:
        return ["GPL-3.0-or-later"]
    markers = (
        ("MIT", "permission is hereby granted, free of charge"),
        ("BSD", "redistribution and use in source and binary forms"),
        ("ISC", "permission to use, copy, modify, and distribute"),
        ("ISC", "permission to use, copy, modify, and/or distribute"),
        ("Unlicense", "released into the public domain"),
        ("CC0-1.0", "cc0 1.0 universal"),
    )
    families = []
    for family, marker in markers:
        if marker in lower:
            families.append(family)
    return families or ["UNCLASSIFIED"]


def add_wintun(entries: list[dict], missing: list[str]) -> None:
    wintun = LOCK["wintun"]
    archive = REPO / ".work" / f"wintun-{wintun['version']}.zip"
    if not archive.exists():
        with urllib.request.urlopen(wintun["url"], timeout=120) as source:
            archive.write_bytes(source.read())
    data = archive.read_bytes()
    if hashlib.sha256(data).hexdigest() != wintun["archive_sha256"]:
        raise SystemExit("Wintun archive checksum mismatch")
    with zipfile.ZipFile(archive) as package:
        dll = package.read("wintun/bin/amd64/wintun.dll")
        license_data = package.read("wintun/LICENSE.txt")
    if hashlib.sha256(dll).hexdigest() != wintun["amd64_dll_sha256"]:
        raise SystemExit("Official Wintun DLL checksum mismatch")
    embedded = (
        Path(next(directory for module, _, directory in module_rows() if module == "github.com/sagernet/sing-tun"))
        / "internal/wintun/amd64/wintun.dll"
    )
    if not embedded.is_file() or hashlib.sha256(embedded.read_bytes()).hexdigest() != wintun["amd64_dll_sha256"]:
        raise SystemExit("Embedded Wintun DLL does not match pinned official archive")
    destination = OUTPUT / "wintun_prebuilt"
    destination.mkdir()
    (destination / "LICENSE.txt").write_bytes(license_data)
    shutil.copyfile(archive, OUTPUT.parent / archive.name)
    entries.append(
        {
            "module": "Wintun prebuilt DLL embedded by sing-tun",
            "version": wintun["version"],
            "artifact_sha256": wintun["amd64_dll_sha256"],
            "source_archive_sha256": wintun["archive_sha256"],
            "licenses": [
                {
                    "name": "LICENSE.txt",
                    "sha256": hashlib.sha256(license_data).hexdigest(),
                    "families": classify(license_data.decode("utf-8", errors="replace")),
                }
            ],
        }
    )


def module_rows() -> list[tuple[str, str, Path]]:
    template = "{{if .Module}}{{.Module.Path}}|{{.Module.Version}}|{{.Module.Dir}}{{end}}"
    env = dict(os.environ, GOOS="windows", GOARCH="amd64", CGO_ENABLED="0")
    output = subprocess.check_output(
        [
            "go",
            "list",
            "-tags",
            ",".join(LOCK["tags"]),
            "-deps",
            "-f",
            template,
            "./cmd/sing-box",
        ],
        cwd=SOURCE,
        env=env,
        text=True,
    )
    rows = set()
    for line in output.splitlines():
        if not line:
            continue
        path, version, directory = line.split("|", 2)
        rows.add((path, version or LOCK["version"], Path(directory)))
    return sorted(rows, key=lambda row: row[0])


def license_files(directory: Path) -> list[Path]:
    found = []
    for name in LICENSE_NAMES:
        candidate = directory / name
        if candidate.is_file():
            found.append(candidate)
    return found


def safe_name(module: str) -> str:
    return module.replace("/", "__").replace(".", "_")


def main() -> None:
    if not SOURCE.is_dir():
        raise SystemExit(f"Pinned source is missing: {SOURCE}")
    if OUTPUT.exists():
        shutil.rmtree(OUTPUT)
    OUTPUT.mkdir(parents=True)

    entries = []
    missing = []
    for module, version, directory in module_rows():
        files = license_files(directory)
        if not files:
            missing.append(module)
            continue
        copied = []
        destination = OUTPUT / safe_name(module)
        destination.mkdir()
        for source in files:
            data = source.read_bytes()
            shutil.copyfile(source, destination / source.name)
            copied.append(
                {
                    "name": source.name,
                    "sha256": hashlib.sha256(data).hexdigest(),
                    "families": classify(data.decode("utf-8", errors="replace")),
                }
            )
        entries.append({"module": module, "version": version, "licenses": copied})

    go_root = Path(subprocess.check_output(["go", "env", "GOROOT"], text=True).strip())
    go_base = go_root if (go_root / "LICENSE").is_file() else go_root.parent
    go_files = license_files(go_base)
    if not go_files:
        missing.append("Go standard library")
    else:
        destination = OUTPUT / "go_standard_library"
        destination.mkdir()
        copied = []
        for source in go_files:
            data = source.read_bytes()
            shutil.copyfile(source, destination / source.name)
            copied.append(
                {
                    "name": source.name,
                    "sha256": hashlib.sha256(data).hexdigest(),
                    "families": classify(data.decode("utf-8", errors="replace")),
                }
            )
        entries.append(
            {
                "module": "Go standard library",
                "version": LOCK["go_version"],
                "licenses": copied,
            }
        )

    add_wintun(entries, missing)

    manifest = {
        "target": "windows/amd64",
        "tags": LOCK["tags"],
        "sing_box_version": LOCK["version"],
        "modules": entries,
        "missing": missing,
    }
    (OUTPUT / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    if missing:
        raise SystemExit("Missing root license files: " + ", ".join(missing))
    print(f"Collected {len(entries)} dependency license entries in {OUTPUT}")


if __name__ == "__main__":
    main()
