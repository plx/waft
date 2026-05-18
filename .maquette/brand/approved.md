# Brand Kit Approval

## Status

Approved.

## Approved Artifact

- Brand board: `.maquette/brand/brand-board-v1.png`
- Generated source image: `/Users/prb/.codex/generated_images/019e12aa-cca4-7b33-9827-20d992de24a7/ig_01669f524252fb3d016a00aed64f908198b9cb2c824c24a958.png`
- Image-worker handoff: used dedicated Maquette image worker `019e12aa-cca4-7b33-9827-20d992de24a7`
- User decision: "Good board let's use it."

## Inspection Notes

The board is a square, readable visual-system board with palette, typography,
spacing, radius, surface, state, and light/dark mode guidance. It uses neutral
labels and does not include a logo, wordmark, mascot, app icon, badge, seal,
monogram, or trademark-like mark.

The board includes one small placeholder command label that reads `wtutil`; it
is not treated as product copy or brand naming. Downstream artifacts use `waft`
only where page content naturally needs the tool name.

## Derived Files

- Design system JSON: `.maquette/brand/design-system.json`
- CSS tokens: `.maquette/brand/tokens.css`

## Token Summary

- Palette: warm paper canvas, white surfaces, graphite text, teal sync accent,
  muted sulfur/amber air-note accent, and red error states.
- Typography: Inter/system UI for prose and JetBrains Mono/ui-monospace for CLI
  and code examples. External font loading is optional; system fallbacks are
  acceptable for the static review page.
- Spacing: 4px base scale with compact documentation rhythm.
- Radius: restrained 2px, 4px, and 8px-first corners, with larger values used
  sparingly.
- Surfaces: low elevation, 1px borders, warm light mode, deep graphite dark
  mode, and high-contrast focus rings.
