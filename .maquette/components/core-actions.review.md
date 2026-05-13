# Core Actions Batch Review

## Source Artifact

- CSS-contract poster: `.maquette/components/component-sheet-core-actions-css-contract-v1.png`
- Transcribed contract: `.maquette/components/contracts/core-actions.contract.css`
- Batch replica: `.maquette/components/core-actions.replica.html`
- Batch CSS: `.maquette/components/css/core-actions.components.css`
- Batch JS: `.maquette/components/js/core-actions.components.js`
- Catalog snapshot: `.maquette/components/core-actions.component-catalog.json`

## Review Mode

Manual review. Automated screenshot, responsive, and schema QA are blocked
because project-local `playwright`, `ajv`, and `ajv-formats` are unavailable,
and this session cannot install network dependencies.

## Fidelity Rubric

- Coverage: 5. All selectors from the core-actions allowlist are represented.
- Visual match: 4. CSS follows the poster contract and approved brand tokens;
  no screenshot comparison was available.
- Anatomy match: 5. Button icon slots, icon buttons, link states, badges, and
  callout icon/body anatomy are preserved.
- Responsive match: 4. Components wrap naturally in the reference; measured
  overflow QA is deferred until browser tooling is available.
- Implementation quality: 4. Semantic controls, token usage, accessible labels,
  and copy enhancement are present.

## Corrections Made

- Raw poster colors were mapped to approved token variables.
- Copy buttons include `data-copy-text` behavior and preserve label recovery.

## Status

Implemented before generating the next component artifact.
