#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "Usage: $0 <directory-with-native-binaries> <output-server.jar>" >&2
  exit 2
fi
native_dir="$(cd "$1" && pwd)"
output="$(mkdir -p "$(dirname "$2")" && cd "$(dirname "$2")" && pwd)/$(basename "$2")"
script_dir="$(cd "$(dirname "$0")" && pwd)"
stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
mkdir -p "$stage/classes/natives"

javac --release 25 -d "$stage/classes" \
  "$script_dir/src/main/java/io/sryza/Bootstrap.java"

found=0
for name in sryzamc-linux-x86_64 sryzamc-linux-aarch64 sryzamc-windows-x86_64.exe; do
  source="$native_dir/$name"
  if [[ ! -f "$source" ]]; then
    if [[ "${SRYZAMC_ALLOW_PARTIAL_NATIVES:-0}" == "1" ]]; then continue; fi
    echo "Missing native engine: $source" >&2
    exit 1
  fi
  cp "$source" "$stage/classes/natives/$name"
  (cd "$stage/classes/natives" && sha256sum "$name" | cut -d ' ' -f 1 > "$name.sha256")
  found=1
done
[[ "$found" == "1" ]] || { echo "No native engines found in $native_dir" >&2; exit 1; }

jar --create --file "$output" --main-class io.sryza.Bootstrap -C "$stage/classes" .
echo "Created $output"
