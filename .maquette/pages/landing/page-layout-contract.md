# Page Layout Contract

Status: approved for implementation.

## Source References

- Page concept: `.maquette/pages/landing/concept.png`
- Brand board: `.maquette/brand/brand-board-v1.png`
- Component catalog: `.maquette/components/component-catalog.json`
- Component contract references:
  - `.maquette/components/component-sheet-core-actions-css-contract-v1.png`
  - `.maquette/components/component-sheet-navigation-layout-css-contract-v1.png`
  - `.maquette/components/component-sheet-docs-composites-css-contract-v2.png`

## Global Layout

- Page max width: standard content width around `72rem`; hero may use the same
  constrained width with two columns.
- Section width behavior: constrained inner content with full-width background
  bands where needed.
- Inline margin rhythm: 24px desktop/tablet, 16px mobile.
- Vertical rhythm: compact documentation rhythm; avoid oversized marketing
  gaps. Top hero should show the next content band on common desktop viewports.
- Desktop breakpoint notes: desktop nav links inline; hero uses content/art
  split plus right navigation behavior rail.
- Tablet breakpoint notes: hero and behavior rail stack; nav links collapse
  behind a menu button near 760px.
- Mobile breakpoint notes: single-column sections, command/code wraps, card
  grids collapse to one column, no page-level horizontal overflow.

## Section Contracts

### Header / Navigation

- Desktop: sticky top nav with text brand left, inline links, active underline,
  theme toggle, and GitHub/source button.
- Tablet: menu button appears; inline links hidden; panel stacks links.
- Mobile closed: compact brand + theme/source/menu actions, no horizontal scroll.
- Mobile open: stacked panel with all links and actions; panel scrolls
  independently if needed.
- Height / density: around 56px.
- Sticky or static behavior: sticky, matching concept.

### Hero

- Target height / min-height: compact first viewport, not full-screen; next
  section should be visible or implied.
- Media aspect and crop: decorative file-transfer art is flexible inline SVG
  and should not create blank bands.
- Text block width: roughly 38rem on desktop; full width on mobile.
- CTA row behavior: command block is the primary action surface; benefit chips
  sit beneath and wrap.
- Mobile stacking: copy, command block, art, then behavior panel.

### Quick Start / Format / Safety

- Target height / compactness: three-column band on desktop with clear dividers;
  stack on mobile.
- Grid or stack: desktop grid `1fr 1fr 1fr`; mobile single column.
- Media aspect and crop: no raster media.
- Component APIs used: `component-step-list`, `component-command`,
  `component-doc-card`, `component-link`, badges and buttons.
- Mobile behavior: command snippets wrap; safety cards stay compact.

### Compatibility Profiles

- Target height / compactness: three equal-height cards with aligned command
  footers.
- Grid or stack: auto-fit equal-height card grid.
- Component APIs used: `component-doc-card-grid`, `component-doc-card--profile`,
  badges, links.
- Mobile behavior: one-column profile cards, footer rows remain bottom-pinned.

### Terminal Footer

- Impact / CTA strip: bottom air-current strip with concise line and back-to-top.
- Newsletter: none in the concept; intentionally omitted.
- Footer: dark surface, brand blurb, docs/resources/community link columns, and
  diagnostic command block.
- Legal / bottom row: MIT/Rust notes in footer blurb; no separate legal bar.
- Target compactness: dense but readable; do not expand into a loose marketing
  footer.
- Mobile behavior: footer columns stack; command block remains readable.

## Image Container Rules

- No major raster image containers are required.
- Decorative file cards and air-current lines are code-rendered SVG/CSS and
  should scale without letterboxing.
- Blank bands or generic placeholder boxes are deviations.

## Deviations Accepted Before Coding

- Deviation: no separate generated logo or raster illustration asset.
- Reason: Maquette brand-kit rules prohibit logo generation, and the concept's
  art can be faithfully represented with inline decorative SVG/CSS.
- Follow-up: if a real logo is later commissioned, integrate it as a separate
  explicit asset task.

## Review Checklist

- Top, middle, and bottom page regions have explicit layout contracts.
- The bottom third of the page is compared against the concept, not just hero.
- Repeated cards have shared anatomy and aligned action rows.
- Footer details are implemented or explicitly recorded as intentional
  deviations.
- Section density and compactness match the concept closely enough for a
  developer documentation site.
