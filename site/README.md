# waft site

Static Astro/Starlight site generated from `static-tool-page-template`.
The renderer input for this project is tracked at `../site-template.json`.

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
