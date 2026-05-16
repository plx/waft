# Component Sheet Inventory

## Planned Batches

The waft landing page needs a compact documentation component library rather
than a broad application UI kit. Batches are processed sequentially.

1. `core-actions`: buttons, icon buttons, links, badges, and compact callouts.
2. `navigation-layout`: responsive site navigation, theme toggle, mobile panel,
   footer links, and section wrappers.
3. `docs-composites`: command blocks, code examples, content cards, profile
   cards, step rows, and equal-height repeated documentation cards.

## Batch: core-actions

- Source artifact: `.maquette/components/component-sheet-core-actions-css-contract-v1.png`
- Artifact type: CSS-contract poster.
- Image-worker handoff: used.
- Inspection result: readable at normal preview size, black-background
  monospace CSS contract, selectors limited to the requested allowlist.
- Visible selector families:
  - `.component-button` with primary, secondary, ghost, copy, small, medium,
    hover, focus-visible, active, and disabled states.
  - `.component-icon-button` and icon sizing.
  - `.component-link` hover and focus behavior.
  - `.component-badge` with sync, air, and error variants.
  - `.component-callout` with safety and note variants plus icon slot.
- Unclear or normalized areas:
  - Raw poster colors are mapped to approved tokens.
  - Poster has minor raw values such as 32px/40px button heights; final
    implementation uses the same compact sizing through component variables.
  - Poster uses callout left borders; final implementation preserves this.
- Decision: implement this batch before generating the next component artifact.

## Batch: navigation-layout

- Source artifact: `.maquette/components/component-sheet-navigation-layout-css-contract-v1.png`
- Artifact type: CSS-contract poster.
- Image-worker handoff: used. The worker rejected an earlier over-broad
  generation and returned a scoped replacement.
- Inspection result: readable square black-background CSS contract. The poster
  includes only the requested selector families, with responsive and focus
  requirements expressed as declarations or adjacent comments.
- Visible selector families:
  - `.component-site-nav` shell with sticky positioning, compact height, glass
    surface, max width, and no horizontal nav scrolling.
  - Brand, link, action, menu button, open panel, active/current link, and theme
    toggle selectors.
  - Section shell and subtle section band.
  - Compact footer layout, link group, and footer link selectors.
- Unclear or normalized areas:
  - The poster references desktop/mobile values in comments; final CSS uses
    media queries around the allowed component selectors.
  - The active link indicator may be border-left or underline; final CSS uses a
    bottom border on desktop and left border in the mobile panel.
- Decision: implement this batch before generating the docs/composites artifact.

## Batch: docs-composites

- Source artifact: `.maquette/components/component-sheet-docs-composites-css-contract-v2.png`
- Rejected artifact: `.maquette/components/component-sheet-docs-composites-css-contract-v1.png`
- Artifact type: CSS-contract poster.
- Image-worker handoff: used.
- Inspection result: v2 is a readable square image-generated CSS contract with
  no `@media` blocks and selector headings scoped to the requested allowlist.
- Rejection notes for v1:
  - It was produced as a deterministic raster render rather than through the
    required image-generation path.
  - It included `@media` blocks outside the strict selector allowlist.
- Visible selector families:
  - `.component-command` with header, body, line, prompt, output, copy control,
    and dark variant.
  - `.component-doc-card` with eyebrow, title, body, footer, safety/profile
    variants, equal-height card behavior, and footer action row pinned to the
    bottom.
  - `.component-doc-card-grid` with equal-height responsive columns and no
    horizontal page overflow.
  - `.component-step-list`, `.component-step`, marker, and content selectors.
- Unclear or normalized areas:
  - Poster omits exact command header border details; final CSS uses approved
    inverse border tokens.
  - Poster comments define responsive behavior without media blocks; final CSS
    uses component-scoped media queries for the replica and page.
- Decision: implement this batch and then assemble the final reusable component
  gallery/catalog.
