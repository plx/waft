# Installation and verification

waft is not published to crates.io. Before the first GitHub release, use a
reviewed source revision or an explicitly chosen Release workflow artifact.
A successful manual build is not evidence that tag validation or GitHub
release publication has run.

## Supported binary targets

| Archive | Target | Build and smoke-test host |
| --- | --- | --- |
| `waft-linux-x86_64.tar.gz` | `x86_64-unknown-linux-gnu` | Ubuntu 24.04 x86_64 |
| `waft-linux-aarch64.tar.gz` | `aarch64-unknown-linux-gnu` | Ubuntu 24.04 ARM64 |
| `waft-macos-x86_64.tar.gz` | `x86_64-apple-darwin` | macOS 15 Intel |
| `waft-macos-aarch64.tar.gz` | `aarch64-apple-darwin` | macOS 15 Apple silicon |

Linux binaries use glibc; musl and older Linux distributions are not verified.
The tested macOS baseline is 15. Archives contain the CLI, license notices,
and documentation. They contain no hook installer. Windows receives source
CI tests but no binary release or managed hook support. Windows overwrite and
permission repair fail per file; Unix overwrite requires the filesystem's
supported publication operations. See [ASSURANCE.md](ASSURANCE.md).

## Install from a reviewed source revision

Use Rust 1.90.0 and a temporary install root to try a pinned commit:

```sh
revision=REVIEWED_COMMIT_SHA
install_root="$(mktemp -d)"
cargo +1.90.0 install --git https://github.com/plx/waft \
  --rev "$revision" --locked --root "$install_root" waft
"$install_root/bin/waft" --version
"$install_root/bin/waft" --help
```

Replace `REVIEWED_COMMIT_SHA` with the full reviewed commit. Omitting `--rev`
installs the current default-branch tip. Keep the install root or copy its
binary to a directory on your `PATH`.

## Download and verify a binary archive

Before publication, download from a chosen successful manual Release run:

```sh
run_id=VERIFIED_RUN_ID
archive=waft-macos-aarch64
mkdir waft-download
cd waft-download
gh run download "$run_id" --repo plx/waft --name "$archive"
```

After a version has been published, download that version's release assets:

```sh
version=v0.1.0
archive=waft-macos-aarch64
gh release download "$version" --repo plx/waft \
  --pattern "$archive.tar.gz" --pattern "$archive.sha256"
```

Choose the archive for your host. Verify both the checksum and the signed
build provenance before extracting or executing it. A checksum alone only
checks that the archive and checksum file agree.

```sh
revision=VERIFIED_COMMIT_SHA
source_ref=refs/heads/main # use refs/tags/v0.1.0 for a published tag build
shasum -a 256 -c "$archive.sha256"
gh attestation verify "$archive.tar.gz" --repo plx/waft \
  --signer-workflow plx/waft/.github/workflows/release.yml \
  --source-digest "$revision" --source-ref "$source_ref" \
  --deny-self-hosted-runners
tar -xzf "$archive.tar.gz"
install_root="$(mktemp -d)"
mkdir -p "$install_root/bin"
install -m 755 "$archive/waft" "$install_root/bin/waft"
"$install_root/bin/waft" --version
"$install_root/bin/waft" --help
```

For a branch build, use the exact ref and commit recorded by the workflow
run. For a published release, use its tag ref and resolved commit. GitHub
artifact attestations establish which workflow and source produced an archive;
they are not a signed Git tag or a guarantee that the code is defect-free.
See the [GitHub CLI verification reference](https://cli.github.com/manual/gh_attestation_verify).

## Optional hooks

Clone and review the same revision in a source checkout. Run `just
install-hooks` there to build and install the pinned binary and managed hooks
for that repository. Run `just uninstall-hooks` from the checkout to restore
the prior configuration. Installation modifies the repository's common Git
directory, not your account-wide Git configuration. It refuses untrusted
worktree hooks and worktree-scoped overrides. Review [README.md](README.md)
and `waft copy --dry-run` before enabling automatic copying.
