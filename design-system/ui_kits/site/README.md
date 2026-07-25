# Waft Site UI Kit

> **Reference implementation:** This kit is the canonical component and
> page-composition reference, not the production application. Port its patterns
> into the repository's top-level `site/`; do not deploy or wholesale-copy this
> React/Babel app.

This kit is plain HTML + React-via-Babel and ships with **one landing page** +
**six doc pages** wired through a single in-page router.

## Design-system inputs

| Design-system file | What it controls in this kit |
|---|---|
| `../../README.md` | Visual foundations, content voice, interaction rules |
| `../../colors_and_type.css` | Canonical `--waft-*` color/type/shape tokens |
| `../../assets/` | Canonical brand assets |
| `../../site-template.json` | Reference copy and information architecture |
| `../../site/src/content/docs/*.mdx` | Reference bodies reproduced in `DocsPage.jsx` |

The paths under `../../site/` are a historical snapshot. The production
Astro/Starlight implementation is the repository's top-level `site/`.

## Using this kit for production

Port the component anatomy, responsive composition, and canonical token use
into Astro/Starlight. Preserve production-only behavior such as the mobile
navigation, Starlight table of contents, Shiki highlighting, base-path routing,
skip links, and automated accessibility coverage.

## Run

Open `index.html` in a browser — no build step. The Babel standalone transforms the JSX files inline.

## Files

```
ui_kits/site/
├── index.html       — page shell + in-page hash router
├── styles.css       — full landing + docs stylesheet
├── Chrome.jsx       — Header + Footer
├── Hero.jsx         — Hero, TerminalCard, TransferPanel
├── Sections.jsx     — FeatureGrid, DocsPreview
└── DocsPage.jsx     — Sidebar + 6 doc-page components
```

## Components

| Component | Purpose |
|---|---|
| `Header` | Sticky nav, brand mark, primary + secondary CTA |
| `Footer` | Theme-aware footer with link list |
| `Hero` | First fold — copy column + terminal/transfer column |
| `TerminalCard` | Reusable theme-aware code block with header + Copy button |
| `TransferPanel` | The right-side "what gets copied" rule preview |
| `FeatureGrid` | Three indexed cards (Quick start / Include format / Safety) |
| `DocsPreview` | Six-up doc-card grid linking into the docs |
| `Sidebar` | Starlight-style guides list |
| `DocsPage` | Sidebar + main column wrapper |
| `UsagePage` etc. | One component per MDX file |

## Navigation

The kit is a single-page click-through. `Header` and every CTA call `onNavigate(screen)`, which sets state and rewrites `location.hash`. Refreshing keeps you on the same screen. Reachable screens:

- `landing` (hero + feature grid + docs preview)
- `usage`, `worktreeinclude`, `safety`, `profiles`, `configuration`, `architecture`

## Known cosmetic differences vs production

- No mobile hamburger toggle (the original has a JS-driven mobile nav panel). Below `900px` the link list hides; the header still works.
- Starlight's auto-generated TOC and right-rail are not reproduced — sidebar-only.
- Code blocks are plain theme-aware monos; no Shiki syntax highlighting.
- No copy-button success animation on terminal (the `aria-live` status text from `landing.ts` is dropped — the button just flips its label).
