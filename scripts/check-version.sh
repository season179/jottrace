#!/usr/bin/env bash
set -euo pipefail

manifest="${JOTTRACE_CARGO_TOML:-Cargo.toml}"
tag="${1:-}"

if [ ! -f "$manifest" ]; then
  echo "version check failed: missing Cargo manifest at ${manifest}" >&2
  exit 1
fi

version="$(sed -n 's/^version = "\([^"]*\)".*/\1/p' "$manifest" | head -n 1)"

if [ -z "$version" ]; then
  echo "version check failed: could not find package version in ${manifest}" >&2
  exit 1
fi

if [[ ! "$version" =~ ^[1-9][0-9]\.([1-9]|1[0-2])\.(0|[1-9][0-9]*)$ ]]; then
  echo "version check failed: Cargo.toml version must use YY.M.PATCH CalVer, got ${version}" >&2
  echo "example: 26.5.0" >&2
  exit 1
fi

if [ -z "$tag" ] && [ "${GITHUB_REF_TYPE:-}" = "tag" ]; then
  tag="${GITHUB_REF_NAME:-}"
fi

if [ -n "$tag" ] && [ "$tag" != "v${version}" ]; then
  echo "version check failed: release tag ${tag} must match Cargo.toml version v${version}" >&2
  exit 1
fi

echo "version check ok: ${version}"
