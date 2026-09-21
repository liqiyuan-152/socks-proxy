#!/usr/bin/env python3
"""Fetch pinned source, verify it, apply the reviewable patch, test and cross-build."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import urllib.request

base = Path(__file__).resolve().parent
repo = base.parents[1]
lock = json.loads((base / 'source-lock.json').read_text())
work = repo / '.work'
output = repo / 'dist' / 'sing-box'
work.mkdir(exist_ok=True)
output.mkdir(parents=True, exist_ok=True)
archive = work / 'sing-box-source.tar.gz'
if not archive.exists():
    with urllib.request.urlopen(lock['source_url'], timeout=120) as src, archive.open('wb') as dst:
        shutil.copyfileobj(src, dst)
if hashlib.sha256(archive.read_bytes()).hexdigest() != lock['source_sha256']:
    raise SystemExit('Source checksum mismatch')
version = subprocess.check_output(['go', 'version'], text=True).split()[2]
if version != lock['go_version']:
    raise SystemExit(f"Expected {lock['go_version']}, found {version}")
with tempfile.TemporaryDirectory(prefix='strict-build-', dir=work) as folder:
    staging = Path(folder)
    with tarfile.open(archive) as tar:
        for entry in tar.getmembers():
            target = (staging / entry.name).resolve()
            if staging.resolve() not in target.parents or entry.issym() or entry.islnk():
                raise SystemExit('Unsafe source archive member')
        tar.extractall(staging)
    source = staging / ('sing-box-' + lock['commit'])
    patches = ['strict-cache.patch', 'all-private.patch']
    for patch in patches:
        subprocess.run(['git', 'apply', '--check', str(base / patch)], cwd=source, check=True)
        subprocess.run(['git', 'apply', str(base / patch)], cwd=source, check=True)
    tags = ','.join(lock['tags'])
    env = dict(os.environ, GOTOOLCHAIN='local')
    subprocess.run(['go', 'test', './experimental/cachefile', '-run', 'TestStrict|TestLegacy', '-count=1'], cwd=source, env=env, check=True)
    subprocess.run(['go', 'test', './route/rule', '-run', 'TestIPAllPrivate', '-count=1'], cwd=source, env=env, check=True)
    win_env = dict(env, GOOS='windows', GOARCH='amd64', CGO_ENABLED='0')
    subprocess.run(['go', 'test', '-c', '-o', str(output / 'cachefile-tests.exe'), './experimental/cachefile'], cwd=source, env=win_env, check=True)
    subprocess.run(['go', 'build', '-trimpath', '-tags', tags, '-ldflags', '-s -w -buildid= -X github.com/sagernet/sing-box/constant.Version=' + lock['version'], '-o', str(output / 'sing-box.exe'), './cmd/sing-box'], cwd=source, env=win_env, check=True)
    with tarfile.open(output / 'patched-source.tar.gz', 'w:gz') as tar:
        tar.add(source, arcname='sing-box-' + lock['version'])
    for name in ['source-lock.json', 'strict-cache.patch', 'all-private.patch', 'LICENSE.upstream', 'build.py', 'license_inventory.py', 'README.md']:
        shutil.copyfile(base / name, output / name)
    subprocess.run(['python3', str(base / 'license_inventory.py')], cwd=repo, env=env, check=True)
    manifest = dict(lock, patch_sha256={name: hashlib.sha256((base/name).read_bytes()).hexdigest() for name in patches}, license_manifest_sha256=hashlib.sha256((output/'licenses'/'manifest.json').read_bytes()).hexdigest(), files={p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in output.iterdir() if p.is_file() and p.name != 'build-manifest.json'})
    (output/'build-manifest.json').write_text(json.dumps(manifest, indent=2)+'\n')
print(output)
