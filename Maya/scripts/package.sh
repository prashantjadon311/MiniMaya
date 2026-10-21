#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
binary=target/release/maya-mini
validation=release/validation.json
[[ -x "$binary" ]] || { echo 'package: release binary is missing' >&2; exit 2; }
[[ -f "$validation" ]] || { echo 'package: validation.json is missing' >&2; exit 2; }
grep -Eq '"release_ready"[[:space:]]*:[[:space:]]*true' "$validation" || { echo 'package: release validation is not ready' >&2; exit 1; }

version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)
[[ -n "$version" ]] || { echo 'package: version is missing' >&2; exit 2; }
name="maya-mini-$version-linux-x86_64"
scratch=$(mktemp -d "${TMPDIR:-/tmp}/maya-package.XXXXXX")
trap 'rm -rf -- "$scratch"' EXIT
mkdir -p "$scratch/$name/bin" "$scratch/$name/share/applications" "$scratch/$name/share/systemd/user" "$scratch/$name/release"
install -m 0755 "$binary" "$scratch/$name/bin/maya-mini"
install -m 0644 README.md CHANGELOG.md "$scratch/$name/"
install -m 0644 packaging/maya-mini.desktop "$scratch/$name/share/applications/"
install -m 0644 packaging/maya-mini.service "$scratch/$name/share/systemd/user/"
install -m 0644 release/validation.json release/acceptance.md "$scratch/$name/release/"
[[ ! -f release/measurements.csv ]] || install -m 0644 release/measurements.csv "$scratch/$name/release/"
(cd "$scratch/$name" && find . -type f -print0 | sort -z | xargs -0 sha256sum >SHA256SUMS)
mkdir -p dist
tar -C "$scratch" -czf "dist/$name.tar.gz" "$name"
sha256sum "dist/$name.tar.gz" >"dist/$name.tar.gz.sha256"
echo "package: wrote dist/$name.tar.gz"
