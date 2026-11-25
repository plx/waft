#!/usr/bin/env bash
# Verify the waft-verified crate with a pinned Verus release.
#
# Usage: scripts/verus-verify.sh [extra verus arguments...]
#
# Downloads the pinned Verus release into VERUS_HOME (default:
# ${XDG_CACHE_HOME:-$HOME/.cache}/waft-verus/<version>), checks its SHA-256,
# installs the Rust toolchain that release requires, and runs
# `cargo verus verify -p waft-verified`. Set VERUS_HOME to an existing
# unpacked release directory to skip the download.
set -euo pipefail

VERUS_VERSION="0.2026.11.22.1ce6a59"
VERUS_TOOLCHAIN="1.100.0"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64)
    asset="verus-${VERUS_VERSION}-x86-linux.zip"
    asset_dir="verus-x86-linux"
    asset_sha256="c955d5958877f4f907cce80aa996d4239a481984829d82e325938c7bf242b0db"
    ;;
  *)
    echo "verus-verify: no pinned Verus release for $(uname -s)-$(uname -m);" >&2
    echo "set VERUS_HOME to an unpacked Verus ${VERUS_VERSION} release" >&2
    if [ -z "${VERUS_HOME:-}" ]; then
      exit 2
    fi
    asset=""
    asset_dir=""
    asset_sha256=""
    ;;
esac

if [ -z "${VERUS_HOME:-}" ]; then
  cache_root="${XDG_CACHE_HOME:-${HOME}/.cache}/waft-verus/${VERUS_VERSION}"
  VERUS_HOME="${cache_root}/${asset_dir}"
  if [ ! -x "${VERUS_HOME}/cargo-verus" ]; then
    mkdir -p "${cache_root}"
    archive="${cache_root}/${asset}"
    url="https://github.com/verus-lang/verus/releases/download/release/${VERUS_VERSION}/${asset}"
    echo "verus-verify: downloading ${url}" >&2
    curl --fail --location --silent --show-error --output "${archive}.partial" "${url}"
    actual="$(sha256sum "${archive}.partial" | cut -d' ' -f1)"
    if [ "${actual}" != "${asset_sha256}" ]; then
      rm -f "${archive}.partial"
      echo "verus-verify: checksum mismatch for ${asset}: ${actual}" >&2
      exit 1
    fi
    mv "${archive}.partial" "${archive}"
    (cd "${cache_root}" && unzip -q -o "${asset}")
  fi
fi

reported="$("${VERUS_HOME}/verus" --version 2>/dev/null | sed -n 's/^ *Version: *//p' || true)"
if [ "${reported}" != "${VERUS_VERSION}" ]; then
  if ! rustup run "${VERUS_TOOLCHAIN}" rustc --version >/dev/null 2>&1; then
    rustup toolchain install "${VERUS_TOOLCHAIN}" --profile minimal
  fi
  reported="$("${VERUS_HOME}/verus" --version | sed -n 's/^ *Version: *//p')"
  if [ "${reported}" != "${VERUS_VERSION}" ]; then
    echo "verus-verify: ${VERUS_HOME} is Verus ${reported}, expected ${VERUS_VERSION}" >&2
    exit 1
  fi
fi

export PATH="${VERUS_HOME}:${PATH}"
cd "${repo_root}"

# Verus artifacts live apart from ordinary builds unless CARGO_TARGET_DIR says
# otherwise. Cargo skips a crate whose fingerprint says it is fresh, and Verus
# then prints nothing, so remove waft-verified's fingerprint to make every run
# verify the crate, and fail unless Verus reports a clean result: a gate that
# silently verified nothing must not pass.
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-${repo_root}/target/verus}"
rm -rf "${CARGO_TARGET_DIR}"/debug/.fingerprint/waft-verified-*
log="$(mktemp)"
trap 'rm -f "${log}"' EXIT
cargo verus verify --locked -p waft-verified --fwd-verus-args-to roots -- --no-cheating "$@" 2>&1 |
  tee "${log}"
if ! grep -Eq '^verification results:: [0-9]+ verified, 0 errors' "${log}"; then
  echo "verus-verify: Verus did not report a clean verification of waft-verified" >&2
  exit 1
fi
