# Repository guidance

## Website work

Before changing `site/`, `site/site-template.json`, or site-related automation:

1. Read `site/design-system/SKILL.md` and `site/design-system/README.md`
   completely.
2. Treat `site/design-system/colors_and_type.css` as the canonical token
   contract. Use `site/design-system/ui_kits/site/` as the component and
   page-composition reference.
3. Use the production assets in `site/design-system/assets/` and retain the
   attribution recorded in `site/design-system/ATTRIBUTION.md`.
4. Port design-system patterns into the Astro/Starlight application. Do not
   replace production behavior with the React/Babel reference kit or the
   reference snapshot under `site/design-system/site/`.
5. Preserve the production site's accessibility, responsive navigation,
   base-path routing, Starlight behavior, and automated checks.
6. Keep terminal, code, footer, and on-page brand surfaces theme-aware through
   the canonical `--waft-code-*` and related tokens.

When a site change intentionally departs from the design system, update the
design-system guidance in the same change and explain the exception.
