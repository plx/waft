#!/usr/bin/env bash
# Install waft's post-checkout integration outside every worktree while
# preserving the repository's previously effective Git hooks.

set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/install-hooks.sh [--uninstall]

Install a reviewed snapshot of waft's hook and binary under the repository's
common Git directory, or restore the hooks configuration that preceded it.

Set WAFT to the executable that should be snapshotted. Without WAFT, the
installer uses `waft` from PATH, then target/release/waft from this checkout.
EOF
}

# Resolve all existing path components physically while allowing the final
# components not to exist yet. This makes containment checks resistant to
# symlink aliases without requiring GNU realpath.
canonicalize_path() {
  local input="$1"
  local probe="${input%/}"
  local suffix=""
  local base
  local parent
  local resolved

  [ -n "${probe}" ] || probe="/"
  while [ ! -e "${probe}" ]; do
    if [ -L "${probe}" ]; then
      return 1
    fi
    base="${probe##*/}"
    parent="${probe%/*}"
    [ -n "${parent}" ] || parent="/"
    [ "${parent}" != "${probe}" ] || return 1
    suffix="/${base}${suffix}"
    probe="${parent}"
  done

  [ -d "${probe}" ] || return 1
  resolved="$(cd "${probe}" && pwd -P)" || return 1
  printf '%s%s\n' "${resolved}" "${suffix}"
}

path_is_within() {
  local candidate="$1"
  local root="$2"
  [ "${candidate}" = "${root}" ] && return 0
  case "${candidate}/" in
    "${root}/"*) return 0 ;;
    *) return 1 ;;
  esac
}

mode="install"
case "${1:-}" in
  "")
    ;;
  --uninstall)
    mode="uninstall"
    ;;
  -h|--help)
    usage
    exit 0
    ;;
  *)
    usage >&2
    exit 2
    ;;
esac

script_dir="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
source_root="$(cd "${script_dir}/.." && pwd -P)"

repo_root_raw="$(git rev-parse --show-toplevel 2>/dev/null)" || {
  echo "install-hooks: run this command inside a non-bare Git worktree." >&2
  exit 1
}
repo_root="$(cd "${repo_root_raw}" && pwd -P)"

common_dir_raw="$(git rev-parse --git-common-dir)"
case "${common_dir_raw}" in
  /*) common_dir_candidate="${common_dir_raw}" ;;
  *) common_dir_candidate="${repo_root}/${common_dir_raw}" ;;
esac
common_dir="$(cd "${common_dir_candidate}" && pwd -P)"
managed_dir="${common_dir}/waft-hooks"
previous_local_file="${managed_dir}/.previous-local-hooks-path"
previous_local_present="${managed_dir}/.previous-local-hooks-path-present"
managed_marker="${managed_dir}/.waft-managed-v1"
hook_names=(
  applypatch-msg pre-applypatch post-applypatch pre-commit pre-merge-commit
  prepare-commit-msg commit-msg post-commit pre-rebase post-checkout post-merge
  pre-push pre-receive update proc-receive post-receive post-update
  reference-transaction push-to-checkout pre-auto-gc post-rewrite
  sendemail-validate fsmonitor-watchman p4-changelist p4-prepare-changelist
  p4-post-changelist p4-pre-submit post-index-change
)

ensure_managed_dir_safe() {
  local managed_real
  local managed_entry

  if [ -L "${managed_dir}" ]; then
    echo "install-hooks: refusing to use a symlink as the managed hook directory:" >&2
    echo "  ${managed_dir}" >&2
    return 1
  fi
  if [ -e "${managed_dir}" ] && [ ! -d "${managed_dir}" ]; then
    echo "install-hooks: managed hook path exists but is not a directory:" >&2
    echo "  ${managed_dir}" >&2
    return 1
  fi
  if [ -d "${managed_dir}" ]; then
    managed_real="$(cd "${managed_dir}" && pwd -P)"
    if [ "${managed_real}" != "${managed_dir}" ]; then
      echo "install-hooks: managed hook directory resolves outside Git metadata:" >&2
      echo "  ${managed_real}" >&2
      return 1
    fi
    for managed_entry in \
      "${managed_dir}"/* \
      "${managed_dir}"/.[!.]* \
      "${managed_dir}"/..?*; do
      if [ -L "${managed_entry}" ]; then
        echo "install-hooks: refusing symlink inside managed hook directory:" >&2
        echo "  ${managed_entry}" >&2
        return 1
      fi
    done
  fi
}

ensure_managed_dir_safe

if [ "${mode}" = "uninstall" ]; then
  current_local="$(git config --local --get core.hooksPath || true)"
  if [ "${current_local}" != "${managed_dir}" ]; then
    echo "install-hooks: refusing to uninstall: core.hooksPath is not managed by waft." >&2
    exit 1
  fi
  if [ ! -f "${managed_marker}" ]; then
    echo "install-hooks: refusing to uninstall an unmarked hook directory:" >&2
    echo "  ${managed_dir}" >&2
    exit 1
  fi

  if [ -f "${previous_local_present}" ]; then
    IFS= read -r previous_local <"${previous_local_file}" || previous_local=""
    git config --local core.hooksPath "${previous_local}"
    echo "Restored the previous local core.hooksPath."
  else
    git config --local --unset-all core.hooksPath || true
    echo "Removed waft's local core.hooksPath override."
  fi

  rm -rf "${managed_dir}"
  exit 0
fi

if [ "$(git config --bool extensions.worktreeConfig 2>/dev/null || true)" = "true" ]; then
  while IFS= read -r -d '' worktree_field; do
    case "${worktree_field}" in
      "worktree "*)
        worktree_path="${worktree_field#worktree }"
        [ -d "${worktree_path}" ] || continue
        if worktree_override="$(
          git -C "${worktree_path}" config --worktree --get core.hooksPath 2>/dev/null
        )"; then
          echo "install-hooks: refusing a worktree-scoped core.hooksPath:" >&2
          echo "  worktree: ${worktree_path}" >&2
          echo "  hooksPath: ${worktree_override}" >&2
          echo "Review it, then remove it from that worktree with:" >&2
          echo "  git -C '${worktree_path}' config --worktree --unset-all core.hooksPath" >&2
          exit 1
        fi
        ;;
    esac
  done < <(git worktree list --porcelain -z)
fi

waft_bin="${WAFT:-}"
if [ -z "${waft_bin}" ] && command -v waft >/dev/null 2>&1; then
  waft_bin="$(command -v waft)"
fi
if [ -z "${waft_bin}" ] && [ -x "${source_root}/target/release/waft" ]; then
  waft_bin="${source_root}/target/release/waft"
fi
if [ -z "${waft_bin}" ] || [ ! -x "${waft_bin}" ]; then
  echo "install-hooks: no executable waft binary found." >&2
  echo "  Build it first, put it on PATH, or set WAFT=/absolute/path/to/waft." >&2
  exit 1
fi
case "${waft_bin}" in
  /*) ;;
  *) waft_bin="$(cd "$(dirname -- "${waft_bin}")" && pwd -P)/$(basename -- "${waft_bin}")" ;;
esac

original_local_present=false
if original_local="$(git config --local --get core.hooksPath 2>/dev/null)"; then
  original_local_present=true
else
  original_local=""
fi
current_local="${original_local}"
effective_hooks_dir="$(
  git rev-parse --path-format=absolute --git-path hooks 2>/dev/null ||
    git rev-parse --git-path hooks
)"
case "${effective_hooks_dir}" in
  /*) ;;
  *) effective_hooks_dir="${repo_root}/${effective_hooks_dir}" ;;
esac
effective_hooks_dir="$(canonicalize_path "${effective_hooks_dir}")" || {
  echo "install-hooks: unable to resolve the effective hooks directory:" >&2
  echo "  ${effective_hooks_dir}" >&2
  exit 1
}

already_installed=false
if [ "${current_local}" = "${managed_dir}" ] ||
  [ "${effective_hooks_dir}" = "${managed_dir}" ]; then
  if [ -f "${managed_marker}" ]; then
    already_installed=true
  else
    echo "install-hooks: core.hooksPath resolves to waft's reserved directory" >&2
    echo "but it is not a recognized waft installation:" >&2
    echo "  ${managed_dir}" >&2
    exit 1
  fi
fi

if [ "${already_installed}" = false ]; then
  # A hook directory in Git's common metadata is safe to chain. Any hook
  # directory inside any checked-out worktree is branch-controlled and is
  # rejected. The exact legacy waft setting is migrated without chaining.
  prior_hooks_dir="${effective_hooks_dir}"
  if [ "${current_local}" = "hooks" ] &&
    [ "${effective_hooks_dir}" = "${repo_root}/hooks" ]; then
    echo "install-hooks: migrating legacy relative hooks/ configuration without chaining it." >&2
    current_local=""
    prior_hooks_dir=""
  elif ! path_is_within "${effective_hooks_dir}" "${common_dir}"; then
    worktree_match=""
    while IFS= read -r -d '' worktree_field; do
      case "${worktree_field}" in
        "worktree "*)
          worktree_path="${worktree_field#worktree }"
          worktree_path="$(canonicalize_path "${worktree_path}")" || continue
          if path_is_within "${effective_hooks_dir}" "${worktree_path}"; then
            worktree_match="${worktree_path}"
            break
          fi
          ;;
      esac
    done < <(git worktree list --porcelain -z)

    if [ -n "${worktree_match}" ]; then
      echo "install-hooks: refusing to chain hooks from a checked-out worktree:" >&2
      echo "  ${effective_hooks_dir}" >&2
      echo "Move reviewed hooks outside every worktree, then retry." >&2
      exit 1
    fi
  fi

  if [ -n "${prior_hooks_dir}" ]; then
    for hook_name in "${hook_names[@]}"; do
      prior_hook="${prior_hooks_dir}/${hook_name}"
      if [ -L "${prior_hook}" ]; then
        echo "install-hooks: refusing to chain a symlinked hook:" >&2
        echo "  ${prior_hook}" >&2
        echo "Use a reviewed regular-file wrapper outside every worktree." >&2
        exit 1
      fi
    done
  fi

  mkdir -p "${managed_dir}"
  ensure_managed_dir_safe
  if [ -n "${current_local}" ]; then
    printf '%s\n' "${current_local}" >"${previous_local_file}"
    : >"${previous_local_present}"
  else
    rm -f "${previous_local_file}" "${previous_local_present}"
  fi
  printf '%s\n' "${prior_hooks_dir}" >"${managed_dir}/.prior-hooks-dir"
else
  prior_hooks_dir=""
  mkdir -p "${managed_dir}"
fi

install -m 0755 "${source_root}/hooks/post-checkout" "${managed_dir}/.waft-post-checkout"
install -m 0755 "${waft_bin}" "${managed_dir}/.waft-bin"

# Proxy every hook name documented by Git so setting core.hooksPath does not
# disable existing commit, push, receive, maintenance, or Git LFS hooks.
for hook_name in "${hook_names[@]}"; do
  install -m 0755 "${source_root}/hooks/dispatcher" "${managed_dir}/${hook_name}"
done
: >"${managed_marker}"

git config --local core.hooksPath "${managed_dir}"

configured_hooks_dir="$(
  git rev-parse --path-format=absolute --git-path hooks 2>/dev/null ||
    git rev-parse --git-path hooks
)"
case "${configured_hooks_dir}" in
  /*) ;;
  *) configured_hooks_dir="${repo_root}/${configured_hooks_dir}" ;;
esac
configured_hooks_dir="$(canonicalize_path "${configured_hooks_dir}")" || true
if [ "${configured_hooks_dir}" != "${managed_dir}" ]; then
  echo "install-hooks: configured hook path is not effective; refusing partial setup." >&2
  echo "  effective: ${configured_hooks_dir:-<unresolved>}" >&2
  if [ "${original_local_present}" = true ]; then
    git config --local core.hooksPath "${original_local}"
  else
    git config --local --unset-all core.hooksPath || true
  fi
  if [ "${already_installed}" = false ]; then
    rm -rf "${managed_dir}"
  fi
  exit 1
fi

# A shared local setting is ineffective in a sibling that has raced in a
# higher-precedence worktree override. Verify every extant worktree, not only
# the checkout from which the installer was invoked.
while IFS= read -r -d '' worktree_field; do
  case "${worktree_field}" in
    "worktree "*)
      worktree_path="${worktree_field#worktree }"
      [ -d "${worktree_path}" ] || continue
      sibling_hooks_dir="$(
        git -C "${worktree_path}" rev-parse --path-format=absolute --git-path hooks 2>/dev/null ||
          git -C "${worktree_path}" rev-parse --git-path hooks
      )"
      case "${sibling_hooks_dir}" in
        /*) ;;
        *) sibling_hooks_dir="${worktree_path}/${sibling_hooks_dir}" ;;
      esac
      sibling_hooks_dir="$(canonicalize_path "${sibling_hooks_dir}")" || true
      if [ "${sibling_hooks_dir}" != "${managed_dir}" ]; then
        echo "install-hooks: managed hooks are not effective in a linked worktree:" >&2
        echo "  worktree: ${worktree_path}" >&2
        echo "  effective: ${sibling_hooks_dir:-<unresolved>}" >&2
        if [ "${original_local_present}" = true ]; then
          git config --local core.hooksPath "${original_local}"
        else
          git config --local --unset-all core.hooksPath || true
        fi
        if [ "${already_installed}" = false ]; then
          rm -rf "${managed_dir}"
        fi
        exit 1
      fi
      ;;
  esac
done < <(git worktree list --porcelain -z)

echo "Installed reviewed waft hook assets in ${managed_dir}"
echo "Set local core.hooksPath to the absolute managed directory."
if [ "${already_installed}" = true ]; then
  echo "Refreshed the installed hook and pinned waft binary; previous hooks remain chained."
elif [ -n "${prior_hooks_dir}" ]; then
  echo "Hooks from ${prior_hooks_dir} remain chained."
fi
