#!/usr/bin/env bash
set -euo pipefail

repo="season179/jottrace"
version="${JOTTRACE_VERSION:-latest}"

if [ -z "${HOME:-}" ]; then
  echo "jottrace installer: HOME is not set" >&2
  exit 1
fi

detect_target() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"

  case "${os}:${arch}" in
    Darwin:arm64) echo "darwin-arm64" ;;
    Darwin:x86_64) echo "darwin-x86_64" ;;
    Linux:x86_64) echo "linux-x86_64" ;;
    Linux:aarch64 | Linux:arm64) echo "linux-arm64" ;;
    *)
      echo "jottrace installer: unsupported platform ${os}/${arch}" >&2
      exit 1
      ;;
  esac
}

release_url() {
  local target artifact
  target="$1"
  artifact="jottrace-${target}.tar.gz"

  if [ -n "${JOTTRACE_RELEASE_BASE_URL:-}" ]; then
    echo "${JOTTRACE_RELEASE_BASE_URL%/}/${version}/${artifact}"
  elif [ "$version" = "latest" ]; then
    echo "https://github.com/${repo}/releases/latest/download/${artifact}"
  else
    echo "https://github.com/${repo}/releases/download/${version}/${artifact}"
  fi
}

target="$(detect_target)"
artifact_url="$(release_url "$target")"
install_dir="${HOME}/.local/bin"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

archive="${tmp_dir}/jottrace.tar.gz"
echo "Downloading jottrace for ${target}..."
curl -fsSL "$artifact_url" -o "$archive"

tar -xzf "$archive" -C "$tmp_dir"
if [ ! -f "${tmp_dir}/jottrace" ]; then
  echo "jottrace installer: artifact did not contain a jottrace binary" >&2
  exit 1
fi

mkdir -p "$install_dir"
install -m 0755 "${tmp_dir}/jottrace" "${install_dir}/jottrace"

echo "Installed jottrace to ${install_dir}/jottrace"

case ":${PATH:-}:" in
  *":${install_dir}:"*) ;;
  *)
    echo
    echo "${install_dir} is not on PATH."
    echo "Add it by pasting this into your shell rc file:"
    echo '  export PATH="$HOME/.local/bin:$PATH"'
    ;;
esac
