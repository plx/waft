# Security policy

## Supported versions

waft has not made a supported release. Security fixes are developed on the
default branch; adopters should pin and review a specific commit.

## Reporting a vulnerability

Do not disclose sensitive vulnerability details in a public issue. Use
GitHub's private vulnerability reporting for this repository when available.
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
