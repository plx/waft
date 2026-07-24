# waft site

This directory contains the production Astro/Starlight site. The renderer input
for its content and information architecture is tracked at
`../site-template.json`.

## Source-of-truth hierarchy

Use these sources in order:

1. `../design-system/README.md` defines the canonical visual language, content
   voice, interaction rules, and asset usage.
2. `../design-system/colors_and_type.css` defines canonical tokens.
   `../design-system/ui_kits/site/` is the component and page-composition
   reference.
3. `../design-system/assets/` contains the canonical brand assets. Preserve the
   attribution in `../design-system/ATTRIBUTION.md`.
4. `../site-template.json` defines shared copy and information architecture,
   while this directory is the deployable Astro/Starlight implementation.

`../design-system/site/` is a reference snapshot, not production code.
`../.maquette/` is an archived, superseded exploration. Neither should
override the canonical design-system files above.

## Migration and change rules

- Read `../design-system/SKILL.md` and `../design-system/README.md` before
  changing the site.
- Import the canonical token contract through `src/styles/theme.css` and use
  the `--waft-*` names directly. Do not duplicate token values in production
  CSS or content configuration.
- Port patterns from the UI kit into Astro. Do not copy its React/Babel router
  or replace Starlight, mobile navigation, accessibility behavior, syntax
  highlighting, or base-path handling with reference-only implementations.
- Keep terminal, code, footer, and on-page brand surfaces theme-aware. Use
  `--waft-code-bg`, `--waft-code-fg`, and related light/dark values rather than
  the legacy fixed-dark `--waft-code` chip token.
- Update both `../site-template.json` and the production implementation when a
  shared content or information-architecture contract changes.
- Record intentional design-system exceptions in the design-system guidance in
  the same change.

## Common commands

```sh
just install
just dev
just check
just test
just build
```

The site is configured for `https://plx.github.io/waft/` with the GitHub Pages base path `/waft`.

The generated Playwright suite runs against mobile, tablet, and desktop projects.
Use `just install-browsers` once locally before `just test`.

## Toolchain

- **Astro 7** (`astro@^7.1.3`) with **Starlight 0.41** (`@astrojs/starlight@^0.41.3`).
- **Node.js 24** (current Active LTS) — pinned via `engines.node` and the CI `node-version`.
- **TypeScript 6.x** for type-checking (`astro check`). This pin is intentional:
  TypeScript 7's native compiler does not yet ship the programmatic API that
  `astro check` (via `@astrojs/language-server`) needs, so Astro projects must
  stay on TypeScript 6 for type-checking until that API arrives in TypeScript 7.1.
  Tracking: https://github.com/withastro/roadmap/discussions/1321 — once
  `@astrojs/check` widens its `typescript` peer range to include `^7`, raise the
  `typescript` devDependency here.
