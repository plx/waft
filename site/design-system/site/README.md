# waft site

> **Reference snapshot:** This directory is retained for design comparison
> only. It is not the production site and is not a source of truth for package
> versions, assets, tokens, or deploy behavior.

This is a historical Astro/Starlight snapshot generated from
`static-tool-page-template`. The snapshot renderer input is tracked at
`../site-template.json`.

Use `../README.md`, `../colors_and_type.css`, and `../ui_kits/site/` for the
canonical design contract. Build and deploy the production application from the
repository's top-level `site/` directory (`../../` from here). Do not copy this
snapshot wholesale into production.

The old package metadata is retained in `package.reference.json` for historical
comparison, rather than as an installable npm manifest. This prevents dependency
automation from treating the archived design reference as a second application.
Do not install its dependencies or use its package versions in production.

This incomplete snapshot is not runnable on its own. See
[`../../README.md`](../../README.md) for supported development,
validation, and deployment commands.
