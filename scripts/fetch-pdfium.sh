#!/usr/bin/env bash
# Fetches the pdfium library the application renders PDF previews with (ADR 0006) for one
# platform into a directory, with its license notices. The release is pinned, and its archive
# must match the SHA-256 recorded here, so CI and release builds use exactly the same library.
#
# Usage: scripts/fetch-pdfium.sh <win-x64|linux-x64|mac-arm64> <directory>
set -euo pipefail

# The pdfium-binaries release matching the pdfium API pdfium-render is built against
# (its `pdfium_8125` feature, which the workspace Cargo.toml enables).
release=8125

platform="${1:?usage: fetch-pdfium.sh <win-x64|linux-x64|mac-arm64> <directory>}"
destination="${2:?usage: fetch-pdfium.sh <win-x64|linux-x64|mac-arm64> <directory>}"

case "$platform" in
  win-x64)
    sha256=9a964261760cff52570aabf038ab346d3a5034dd8b30d0d5fe8861235d356b8b
    library=bin/pdfium.dll
    ;;
  linux-x64)
    sha256=e1c3ce6e270cb5e0c20816b379ad9f2c1309972697c505f508de92bf5e7d6659
    library=lib/libpdfium.so
    ;;
  mac-arm64)
    sha256=4939bd935ac83ddc09a3a4f038bc43a19cea1660316a3667f26cb07a070e139d
    library=lib/libpdfium.dylib
    ;;
  *)
    echo "no pinned pdfium for platform $platform" >&2
    exit 1
    ;;
esac

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
archive="$work/pdfium.tgz"
curl --fail --silent --show-error --location --retry 3 --output "$archive" \
  "https://github.com/bblanchon/pdfium-binaries/releases/download/chromium/$release/pdfium-$platform.tgz"

if command -v sha256sum > /dev/null; then
  actual="$(sha256sum "$archive" | cut -d ' ' -f 1)"
else
  actual="$(shasum -a 256 "$archive" | cut -d ' ' -f 1)"
fi
if test "$actual" != "$sha256"; then
  echo "pdfium-$platform.tgz has SHA-256 $actual, not the pinned $sha256" >&2
  exit 1
fi

mkdir -p "$work/pdfium" "$destination"
tar -xzf "$archive" -C "$work/pdfium"
cp "$work/pdfium/$library" "$destination/"
cp "$work/pdfium/LICENSE" "$destination/PDFIUM-LICENSE"
if test -d "$work/pdfium/licenses"; then
  rm -rf "$destination/pdfium-licenses"
  cp -R "$work/pdfium/licenses" "$destination/pdfium-licenses"
fi
echo "pdfium chromium/$release for $platform in $destination"
