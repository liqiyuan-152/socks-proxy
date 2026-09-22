#!/usr/bin/env bash
set -euo pipefail

project_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
output_dir=${1:-"$project_dir/.work/windows-installer/package"}
release_exe="$project_dir/target/x86_64-pc-windows-msvc/release/socks-proxy.exe"
core_exe="$project_dir/dist/sing-box/sing-box.exe"

require_file() {
  if [[ ! -f "$1" ]]; then
    printf 'missing required file: %s\n' "$1" >&2
    exit 1
  fi
}

for source in \
  "$release_exe" \
  "$core_exe" \
  "$project_dir/docs/user-guide.md" \
  "$project_dir/docs/validation/dependency-licenses.md" \
  "$project_dir/dist/sing-box/patched-source.tar.gz" \
  "$project_dir/dist/sing-box/build.py" \
  "$project_dir/dist/sing-box/source-lock.json" \
  "$project_dir/dist/sing-box/all-private.patch" \
  "$project_dir/dist/sing-box/strict-cache.patch" \
  "$project_dir/dist/sing-box/licenses/manifest.json"; do
  require_file "$source"
done

rm -rf "$output_dir"
mkdir -p "$output_dir/licenses" "$output_dir/source"
cp "$release_exe" "$output_dir/socks-proxy.exe"
cp "$core_exe" "$output_dir/sing-box.exe"
cp "$project_dir/docs/user-guide.md" "$output_dir/user-guide.md"
cp "$project_dir/docs/validation/dependency-licenses.md" "$output_dir/dependency-licenses.md"
cp -R "$project_dir/dist/sing-box/licenses/." "$output_dir/licenses/"
cp "$project_dir/dist/sing-box/patched-source.tar.gz" "$output_dir/source/"
cp "$project_dir/dist/sing-box/build.py" "$output_dir/source/"
cp "$project_dir/dist/sing-box/source-lock.json" "$output_dir/source/"
cp "$project_dir/dist/sing-box/all-private.patch" "$output_dir/source/"
cp "$project_dir/dist/sing-box/strict-cache.patch" "$output_dir/source/"

(
  cd "$output_dir"
  shasum -a 256 socks-proxy.exe sing-box.exe user-guide.md dependency-licenses.md \
    licenses/manifest.json source/patched-source.tar.gz source/build.py \
    source/source-lock.json source/all-private.patch source/strict-cache.patch > SHA256SUMS
)
printf 'prepared installer input: %s\n' "$output_dir"
