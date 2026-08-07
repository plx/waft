---
name: waft-design
description: Use this skill to generate well-branded interfaces and assets for waft (a Rust CLI for copying .worktreeinclude-selected ignored files between Git worktrees), either for production or throwaway prototypes/mocks/etc. Contains essential design guidelines, colors, type, fonts, assets, and UI kit components for prototyping.
user-invocable: true
---

Read the `README.md` file within this skill completely, then explore the files
relevant to the task. This directory is the canonical visual and content-voice
system for the **waft documentation website**. The deployable Astro + Starlight
application lives in the parent directory, the repository's top-level `site/`.

Key files in this skill:

- `README.md` — full content + visual foundations, iconography rules, index.
- `colors_and_type.css` — drop-in CSS custom properties and semantic styles.
- `assets/` — brand mark, wordmark, favicon (the **only** icons the brand owns).
- `preview/` — single-concept reference cards for tokens and components.
- `ui_kits/site/` — pixel-faithful recreation of the docs site (landing + 6 doc pages) as plain HTML + React/Babel. Use its component anatomy as a reference and port it into the target stack.
- `site/` — historical source snapshot for cross-reference only; never treat it
  as the production application or copy its package versions wholesale.
- `ATTRIBUTION.md` — source-artwork provenance and the attribution that must be
  retained with redistributed adaptations.

If creating visual artifacts (slides, mocks, throwaway prototypes), copy assets
out of this skill and create static HTML files for the user to view; load
`colors_and_type.css` for tokens. For production work, port the canonical
tokens, assets, and UI-kit patterns into the parent Astro/Starlight application
while preserving its accessibility, navigation, routing, and documentation
behavior.

If the user invokes this skill without any other guidance, ask them what they want to build or design (a new docs page? a marketing variant? a release-notes layout?). Then ask 2–3 clarifying questions about scope and audience, and act as an expert designer who outputs HTML artifacts _or_ production code, depending on the need.

Hard rules the brand insists on:

- **No emoji. No exclamation marks. No hedging.** The voice is declarative, technical, second-person.
- **No imagery, illustrations, or gradients.** The terminal card and reference tables are the visual interest.
- **No icon library.** The brand owns one SVG — the mark. If you must add a UI icon (e.g. copy button, hamburger), use Lucide via CDN and match its 2px stroke style.
- **Code surfaces are theme-aware:** use `--waft-code-bg`,
  `--waft-code-fg`, and `--waft-code-btn`. `--waft-code` is the legacy
  fixed-dark value for favicon and fixed-chip contexts only.
- **Hover/focus pattern is uniform:** 8% accent wash plus an opaque
  `--waft-focus` outline.
