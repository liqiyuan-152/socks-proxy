#!/usr/bin/env python3
"""Run a UTF-8 PowerShell file over an existing SSH connection (no stored secrets)."""
import argparse
import base64
from pathlib import Path
import subprocess

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('host', help='SSH alias or user@host')
p.add_argument('script', type=Path)
p.add_argument('--socket', help='Existing SSH control socket')
a = p.parse_args()
prefix = "[Console]::OutputEncoding=[Text.Encoding]::UTF8; $ProgressPreference='SilentlyContinue'; $ErrorActionPreference='Stop';\n"
encoded = base64.b64encode((prefix + '& {\n' + a.script.read_text(encoding='utf-8') + '\n}').encode('utf-16le')).decode('ascii')
cmd = ['ssh', '-o', 'BatchMode=yes', '-o', 'ConnectTimeout=10']
if a.socket:
    cmd += ['-S', a.socket]
cmd += [a.host, 'powershell -NoProfile -NonInteractive -EncodedCommand ' + encoded]
raise SystemExit(subprocess.call(cmd))
