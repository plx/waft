# Concept Region Inventory

Page: `landing`
Concept image: `.maquette/pages/landing/concept.png`

Every visible concept region defaults to implementation.

| Region | Visible concept details | Status | Implementation notes / reason |
| --- | --- | --- | --- |
| Header/nav | Text brand at left, desktop inline links, theme controls, GitHub button, active underline | implemented | Use `component-site-nav`, `component-theme-toggle`, component buttons, and text brand. |
| Navigation behavior callout | Desktop compact nav, mobile collapsed bar, mobile expanded stacked panel, no horizontal scrolling note | implemented | Include visible nav behavior panel in hero-side rail, plus actual responsive nav behavior in code. |
| Hero | Large `waft`, value proposition, short odor/air pun line, command block, file-transfer line art, three benefit chips | implemented | Use command component, inline SVG/CSS file-transfer motif, and badges/chips. |
| Quick start | Five numbered steps and two command snippets with usage link | implemented | Use `component-step-list`, compact command tiles, and component link. |
| `.worktreeinclude` format | Code-like example with comments, include/negation patterns, format reference CTA | implemented | Use command/code styling with wrapped lines and copy affordance. |
| Safety guarantees | Four red-tinted safety cards: preview first, path validation, never copies tracked files, overwrite behavior | implemented | Use `component-doc-card` with safety variant and inline icons. |
| Compatibility profiles | Section heading, three equal-height cards for `claude`, `git`, `wt`, badges/tags, command footer rows | implemented | Use `component-doc-card-grid`, profile variants, bottom-pinned footers. |
| Footer | Dark terminal-style footer with brand blurb, docs/resources/community link columns, diagnostic command block, bottom air-current strip and back-to-top link | implemented | Implement as specific terminal footer, not generic link list. |
| Decorative media | Air currents, dotted arrow, file cards, footer air strip | implemented differently with reason | Code-rendered with inline SVG/CSS to avoid generating a logo or new raster assets. |

## Notes

- The concept uses no product photography or required raster media beyond the
  concept screenshot itself.
- The line-art file-transfer motif is decorative and should remain secondary to
  the documentation content.
