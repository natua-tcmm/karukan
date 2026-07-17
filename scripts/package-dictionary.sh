#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 1 || $# -gt 2 ]]; then
  echo "usage: $0 DICT_BIN [OUTPUT_TGZ]" >&2
  exit 2
fi

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
dict_bin="$1"
output="${2:-dict.tgz}"

if [[ ! -f "$dict_bin" ]]; then
  echo "dictionary not found: $dict_bin" >&2
  exit 1
fi

cargo run --quiet --manifest-path "$repo_root/Cargo.toml" \
  --release --bin karukan-dict-sources -- \
  verify "$repo_root/dictionary-sources.toml"

if grep -Eq '^[[:space:]]*sources[[:space:]]*=[[:space:]]*\[[[:space:]]*\][[:space:]]*$' \
  "$repo_root/dictionary-sources.toml"; then
  echo "dictionary-sources.toml has no enabled sources" >&2
  exit 1
fi

staging="$(mktemp -d)"
trap 'rm -rf "$staging"' EXIT

cp "$dict_bin" "$staging/dict.bin"
cp "$repo_root/dictionary-sources.toml" "$staging/dictionary-sources.toml"
cp "$repo_root/docs/dictionary-licenses.md" "$staging/DICTIONARY_LICENSES.md"

(
  cd "$staging"
  shasum -a 256 dict.bin dictionary-sources.toml DICTIONARY_LICENSES.md > SHA256SUMS
)

tar czf "$output" -C "$staging" \
  dict.bin dictionary-sources.toml DICTIONARY_LICENSES.md SHA256SUMS
echo "wrote $output"
