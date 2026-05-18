# Component Sheet Implementation Log

## QA Tooling

`shared/scripts/ensure-qa-tooling.mjs --project . --check-browser` reported
missing project-local `playwright`, `ajv`, and `ajv-formats`. This session has
restricted network access and no approval path for installs, so automated
browser screenshots, responsive measurements, and schema validation are blocked.
Linked-asset checks and JSON syntax checks were run where possible.

## Batch: core-actions

- Source artifact: `.maquette/components/component-sheet-core-actions-css-contract-v1.png`
- Contract: `.maquette/components/contracts/core-actions.contract.css`
- Replica: `.maquette/components/core-actions.replica.html`
- CSS: `.maquette/components/css/core-actions.components.css`
- JS: `.maquette/components/js/core-actions.components.js`
- Catalog snapshot: `.maquette/components/core-actions.component-catalog.json`
- Review: `.maquette/components/core-actions.review.md`
- Review evidence: manual source/image inspection and linked-asset validation
- Rubric: coverage 5, visual 4, anatomy 5, responsive 4, implementation 4
- Corrections: raw poster values mapped to approved tokens
- Status: implemented before next sheet

## Batch: navigation-layout

- Source artifact: `.maquette/components/component-sheet-navigation-layout-css-contract-v1.png`
- Contract: `.maquette/components/contracts/navigation-layout.contract.css`
- Replica: `.maquette/components/navigation-layout.replica.html`
- CSS: `.maquette/components/css/navigation-layout.components.css`
- JS: `.maquette/components/js/navigation-layout.components.js`
- Catalog snapshot: `.maquette/components/navigation-layout.component-catalog.json`
- Review: `.maquette/components/navigation-layout.review.md`
- Review evidence: manual source/image inspection and linked-asset validation
- Rubric: coverage 5, visual 4, anatomy 5, responsive 4, implementation 4
- Corrections: added open mobile nav example and independent panel scrolling
- Status: implemented before next sheet

## Batch: docs-composites

- Rejected artifact: `.maquette/components/component-sheet-docs-composites-css-contract-v1.png`
- Accepted source artifact: `.maquette/components/component-sheet-docs-composites-css-contract-v2.png`
- Contract: `.maquette/components/contracts/docs-composites.contract.css`
- Replica: `.maquette/components/docs-composites.replica.html`
- CSS: `.maquette/components/css/docs-composites.components.css`
- JS: `.maquette/components/js/docs-composites.components.js`
- Catalog snapshot: `.maquette/components/docs-composites.component-catalog.json`
- Review: `.maquette/components/docs-composites.review.md`
- Review evidence: manual source/image inspection and linked-asset validation
- Rubric: coverage 5, visual 4, anatomy 5, responsive 4, implementation 4
- Corrections: rejected noncompliant v1 and regenerated compliant v2
- Status: implemented before final gallery

## Final Gallery

- Combined gallery: `.maquette/components/replica-gallery.html`
- Combined CSS: `.maquette/components/css/components.css`
- Combined JS: `.maquette/components/js/components.js`
- Linked-asset check: passed
