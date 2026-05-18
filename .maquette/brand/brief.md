# waft Landing Page Brand Brief

## Product Summary

`waft` is a minor but useful CLI for software developers who use Git worktrees.
It copies ignored local files selected by a `.worktreeinclude` file from one
worktree to another, so local config, secrets, and build-adjacent files can move
without tracking them in Git.

The name expands to "worktree-aware file tool" and also suggests files being
wafted across worktrees. The page may use a small humorous odor/air-current
motif, but the core impression should remain competent, careful, and developer
trustworthy.

## Audience

- Software developers already familiar with Git, `.gitignore`, and worktrees.
- CLI users who want quick installation, command examples, and exact behavior.
- Maintainers evaluating whether the tool is safe around untracked, ignored,
  and potentially sensitive files.

## Tone

- Clean and terse, like a polished developer documentation site.
- Quietly funny, with humor limited to small visual details and a short line.
- Precise, safety-conscious, and practical.
- Minimal, typographically strong, and comfortable in light and dark mode.

## Visual Direction

- Developer-site typography: strong monospace accents for commands and paths,
  paired with a readable sans-serif for prose.
- Sane scale hierarchy: compact header, useful hero, visible quick-start content
  above the fold, documentation sections below.
- Color should avoid generic neon terminal styling. Use a restrained neutral
  base with one distinctive accent and one earthy/comic secondary color that can
  support the odor/air motif without dominating the page.
- The page may include a simple line-art illustration of files or config sheets
  being carried by curving air lines. Avoid a mascot or trademark-like logo in
  the brand-board phase.

## Content Priorities

- One-line explanation: copy `.worktreeinclude`-selected ignored files between
  Git worktrees.
- Quick-start command examples.
- `.worktreeinclude` format example.
- Safety guarantees: tracked files are never overwritten, symlink traversal is
  blocked, writes are atomic, and dry runs are mutation-free.
- Compatibility profiles: `claude`, `git`, and `wt`.
- Clear calls to read docs, install, and view source.

## Constraints

- All Maquette outputs must remain under `.maquette/`.
- Do not create or overwrite a root-level `index.html`.
- Brand-board generation must not create a logo, wordmark, mascot, app icon, or
  trademark-like mark.
- The final page should support light and dark modes.
- The final page should be static and self-contained enough for review.

## Accessibility Requirements

- Meet WCAG AA contrast in both light and dark modes.
- Preserve visible focus states for navigation, buttons, and copyable command
  controls.
- Avoid horizontal overflow at common mobile/tablet/desktop widths.
- Do not rely on color alone to distinguish important command or safety states.
