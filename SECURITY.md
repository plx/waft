# Security policy

## Supported versions

Only the latest published release is supported, currently
[v0.1.0](https://github.com/plx/waft/releases/tag/v0.1.0). Security fixes are
developed on the default branch and shipped in a new release. Source adopters
should pin and review a specific commit.

## Reporting a vulnerability

Do not disclose sensitive vulnerability details in a public issue. Use
[GitHub's private vulnerability reporting](https://github.com/plx/waft/security/advisories/new)
for this repository.
If that option is unavailable, open a minimal issue requesting a private
contact channel without including exploit details or secrets.

Include the affected revision, operating system and filesystem, a minimal
reproduction, and the impact you observed. You should receive an
acknowledgement within seven days.

## Operational guidance

waft copies ignored files and may therefore duplicate credentials. Prefer
short-lived credentials or secret injection, keep `.worktreeinclude`
selections narrow, and inspect `waft copy --dry-run` before enabling
automation.

Only install the optional Git hook from a reviewed revision. Never configure
`core.hooksPath` to a directory inside a checked-out worktree.
