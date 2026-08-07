# Waft Design System

A small, opinionated design system for **waft** — a Rust CLI tool for copying `.worktreeinclude`-selected ignored files between Git worktrees.

This directory is the canonical visual and content-voice system for the **waft
documentation site** (the production Astro + Starlight application in the
parent directory). The CLI itself ships as a terminal binary; everything here
applies to the *web surface* around it: the landing page and the docs.

## Authority and references

Use the files in this directory in this order:

- **Foundations and content voice:** this `README.md`.
- **Canonical tokens:** [`colors_and_type.css`](colors_and_type.css).
- **Canonical assets:** [`assets/`](assets/), with provenance recorded in
  [`ATTRIBUTION.md`](ATTRIBUTION.md).
- **Component and page reference:** [`ui_kits/site/`](ui_kits/site/).
- **Production implementation:** [`../`](../).
- **Copywriting and information architecture:**
  [`../site-template.json`](../site-template.json).
- **Historical source snapshot:** [`site/`](site/) inside this directory,
  retained for comparison only.

The production site implements this contract; it does not supersede it. Start
new visual work here, then port the result into Astro/Starlight while preserving
production accessibility, navigation, routing, and documentation behavior.

---

## What waft is

> `waft` copies `.worktreeinclude`-selected ignored files between Git worktrees.

When you use `git worktree`, local configuration files (`.env`, API keys, build caches) live in `.gitignore` and don't carry over to linked worktrees. waft solves that: drop a `.worktreeinclude` file listing what you want copied, and `waft` does the rest.

It's a single-binary Rust CLI. The product surface is the *terminal*. The web surface is one landing page plus six MDX docs (`usage`, `worktreeinclude`, `safety`, `profiles`, `configuration`, `architecture`).

### Product surfaces this system covers

| Surface | Source | Status |
|---|---|---|
| **Documentation website** | `../` (Astro + Starlight) | Covered — see `ui_kits/site/` |
| CLI itself | `../../src/` (Rust) | Out of scope — no GUI |

---

## Content Fundamentals

The voice of waft's docs is **declarative, technical, second-person, and code-first**. It reads like the man page for a well-loved Unix tool.

### Tone & casing

- **Second person ("you").** Not "we." Not "users." `"Use this profile when you want Git-style per-directory exclude semantics."`
- **Imperative for instructions.** `"Add .worktreeinclude to the source repo."` / `"Run waft copy --dry-run."`
- **Sentence case for headings.** `"Quick start"`, not `"Quick Start"`. (Page titles are also sentence case: `Architecture`, `Configuration`, `.worktreeinclude`.)
- **Backticks are mandatory** for: command names (`waft`), flag names (`--dry-run`), file names (`.worktreeinclude`, `.waft.toml`), config keys (`compat.profile`), engine identifiers (`claude-2026-04`, `wt-0.39`), and crate names (`gix`).
- **No emoji. No exclamation marks. No hedging.** Statements are stated, not softened.
- **Bold for guarantees and contracts** in safety contexts: `**Tracked files are never overwritten.**`

### Sentence shape

Short. Imperative or declarative. Often a single fact per sentence. Where a concept has a precise definition, the docs spell it out as a numbered list of necessary conditions:

> A file is eligible for copying only when all of these are true:
> 1. It exists in the source worktree.
> 2. It is a regular file, not a symlink, directory, or special file.
> 3. It is selected by the active compatibility profile.
> 4. Git confirms it is ignored and untracked.
> 5. It is not dropped by the active exclusion set.

### Page anatomy

Every docs page follows the same beats:

1. **One-line definition** of what the topic is (no preamble).
2. **A code block** showing the simplest invocation, often within the first 200 words.
3. **A reference table** for commands, options, or profiles when applicable.
4. **Numbered or bulleted enumerations** for invariants, eligibility rules, pipeline steps.

The landing hero uses the eyebrow `Git worktree file copier`, the headline
`Copy ignored files.`, a direct definition of what `waft` copies, and the
`Usage` / `Source` actions. The actions follow the body copy directly. Do not
add a project-metadata badge row to the hero.

### Vocabulary

The system has its own terms; respect them:

- **worktree** — the canonical concept, never abbreviated.
- **eligible / eligibility** — the formal predicate for what gets copied.
- **profile** (`claude` / `git` / `wt`) — a coordinated bundle of behavior.
- **compat-profile** — the CLI flag form.
- **plan, then execute** — the safety model. Always paired.
- **dry-run** — hyphenated.
- **tracked / ignored / untracked** — Git terms, used precisely.
- **engine** / **semantics** — the matcher implementation (`claude-2026-04`, `git`, `wt-0.39`).

### What it never does

- Never markets ("revolutionary," "powerful," "delightful").
- Never says "easy" or "simple" — shows the thing instead.
- Never apologizes for friction (`--dry-run` is presented as a virtue, not a hurdle).
- Never uses screenshots of the terminal — actual code blocks only.

---

## Visual Foundations

### Color system

A two-accent palette on a near-white surface, with a theme-aware code surface (light in light mode, dark in dark mode).

> **Token prefix.** This system exposes canonical tokens as `--waft-*` to avoid
> clashing with downstream code. The production site imports and uses these
> names directly.

| Token | Light | Dark | Used for |
|---|---|---|---|
| `--waft-accent` | `#16a085` (teal) | `#8fd7cc` | Primary brand stroke, link hovers |
| `--waft-accent-dark` | `#0f6e61` | `#8fd7cc` | Link text, eyebrows, primary button |
| `--waft-accent-2` | `#d6c24a` (mustard) | `#d6c24a` | Secondary stroke on the logo, micro-accent |
| `--waft-accent-soft` | `#dff5f1` | `rgb(22 160 133 / 16%)` | Tinted hover background |
| `--waft-air-soft` | `#fff8c9` | `rgb(214 194 74 / 16%)` | Mustard wash |
| `--waft-focus` | `#0f6e61` | `#8fd7cc` | Opaque keyboard-focus ring |
| `--waft-ink` | `#1f2328` | `#f8fafc` | Body text |
| `--waft-ink-soft` | `#3d4650` | `#cbd5dc` | Secondary text |
| `--waft-muted` | `#6b7280` | `#93a0aa` | Tertiary / meta |
| `--waft-surface` | `#f7f8f6` | `#101512` | Page background |
| `--waft-panel` | `#ffffff` | `#19201d` | Cards, header, menus |
| `--waft-subtle` | `#eef2f1` | `#141a17` | Inset fills |
| `--waft-line` | `#e2e5e9` | `rgb(248 250 252 / 14%)` | Hairlines |
| `--waft-line-strong` | `#c8cdd3` | `rgb(248 250 252 / 22%)` | Dividers |
| `--waft-code` | `#151a1f` | `#151a1f` | Legacy fixed-dark chip (favicon, wordmark tiles) |
| `--waft-code-bg` | `#f3f5f4` | `#151a1f` | Terminal / code-block surface (theme-aware) |
| `--waft-code-fg` | `#1f2328` | `#f8fafc` | Code text (mirrors `--waft-ink`) |
| `--waft-code-neg` | `#b42323` | `#fca5a5` | Negation / error inside code |
| `--waft-mark-2` | `#a8871f` | `#d6c24a` | Brand-mark inner Y stroke (theme-aware mustard) |
| `--waft-error` | `#b42323` | `#fca5a5` | Negation example, danger |

**Vibe.** Cool but warm. The teal is workshop-green (engineering, GitHub-y), not Slack-blue or marketing-aqua. Mustard provides a deliberately analog second stroke — it reads like a printer's accent ink on technical drawings. The page is almost-white (`#f7f8f6`), never pure paper. Code surfaces, the footer, and the on-page brand mark are theme-aware: uniformly light in light mode, uniformly dark in dark mode — no dark panels leaking into the light theme.

### Typography

Two families. No optional third.

- **Sans:** `Inter` → fallbacks to `ui-sans-serif`, `system-ui`, `-apple-system`, `BlinkMacSystemFont`, `Segoe UI`, `sans-serif`.
- **Mono:** `JetBrains Mono` → fallbacks to `SFMono-Regular`, `Consolas`, `Liberation Mono`, `monospace`.

**Weight strategy.** Heavy at the brand level, light in body:
- Brand wordmark + h1 + eyebrows: **800 (Black)**.
- h2/h3, links, buttons, badges: **700 (Bold)**.
- Lede: **500**.
- Body: **400**.

**Scale.** `clamp()`-driven, fluid for display sizes:
- Display (hero h1): `clamp(3rem, 8vw, 6rem)` with `line-height: 0.95`.
- h2: `clamp(1.75rem, 4vw, 2.75rem)`.
- h3 / card title: `1.2rem`.
- Lede: `1.125rem`.
- Body: `1rem` at `line-height: 1.5`.
- Small (button/nav): `0.875rem`.
- Eyebrow/badge: `0.75rem`, uppercased, often in `--waft-font-mono`.

**Treatments.** Eyebrows are uppercase mono-ish but rendered in Inter Black;
status and tag badges are mono, 12px, bordered pills.

> **Font substitution flag.** The site uses Inter and JetBrains Mono from system / CDN; we pull both from Google Fonts in `colors_and_type.css` to match the production build. If you have licensed/self-hosted copies you'd like to vendor, drop them in `fonts/` and swap the `@import`.

### Spacing & layout

- **Container:** `min(1220px, calc(100% - 3rem))` centered. The site rarely goes wider.
- **Section rhythm:** `padding-block: 3.5rem;` (`section`) / `padding-block: 4rem 2rem;` (hero top).
- **Inline gaps:** the system loves `gap` on flex/grid — typical values are `0.5rem`, `0.75rem`, `1rem`, `1.5rem`, `2.5rem`.
- **Hero grid:** `grid-template-columns: minmax(0, 0.9fr) minmax(520px, 1.1fr)` — copy slightly narrower than terminal card.
- **Feature grid:** `repeat(3, minmax(0, 1fr))` with `gap: 0.75rem`. Collapses to 1 column at `900px`.

### Backgrounds

No imagery. No illustrations. No gradients. No textures.

The site is flat: a near-white surface, white panels, and a code surface that tracks the theme. The only "image" content in the entire site source is the **logo SVG** (a boxed computer-fan mark) and the **favicon** (same).

What replaces imagery as visual interest: **the terminal command card**
(theme-aware code surface with mono text), and **dense reference tables**.
Those *are* the hero.

### Borders, radii, shadows

- **Radius:** `8px` (`--waft-radius`) for cards, panels, terminal; `4px` (`--waft-radius-sm`) for buttons, badges, nav links.
- **Borders:** 1px hairlines, almost always `var(--waft-line)`. Strong dividers use `--waft-line-strong`. Dashed `1px` on a hero divider (`border-left: 1px dashed var(--waft-line-strong)`).
- **Shadows:** Minimal. The system shadow is `0 1px 2px rgb(16 24 40 / 6%)` — barely there, just enough to lift a card off the surface. The mobile-nav pop-out goes to `0 8px 24px rgb(31 35 40 / 12%)`. **No drop shadows on text. No glow. No inner shadow.**
- **Cards:** white panel, 1px hairline border, 8px radius, subtle shadow. That's it. No colored borders, no left-accent stripes, no gradient frames.

### Buttons

Three variants, all share the same skeleton (`min-height: 36px`, `4px` radius, `0.875rem/1` bold sans, subtle shadow):

- **`.button--primary`** — solid `--waft-accent-dark` background, matching border, and theme-aware `--waft-surface` text.
- **`.button--secondary`** — surface bg, ink text, hairline border.
- **`.button--ghost`** — same as secondary, used for tertiary actions.
- **`.copy-button`** — sits inside the terminal card, using the code-surface tokens (`--waft-code-btn` fill, `--waft-code-fg` text), mono font.

### Interaction states

The site uses a uniform hover/focus pattern across nav, footer, and links:

```css
hover/focus-visible {
  background: color-mix(in srgb, var(--waft-accent) 8%, transparent);
  outline:   2px solid var(--waft-focus);
  outline-offset: 2px;
}
```

So: **8% accent wash + an opaque, theme-aware focus ring**. Calm,
accessible, brand-tinted. No animation, no scale, no lift. Active/press states
fall back to platform defaults.

### Animation

**Almost none.** `scroll-behavior: smooth` on `html`, and a `prefers-reduced-motion` block that nukes everything to `0.01ms`. No entrance animations, no scroll-triggered reveals, no hover transitions on shadows. The aesthetic is deliberately static — this is a CLI tool's docs, not a SaaS landing page.

### Transparency & blur

Used in exactly one place: the sticky header.

```css
.site-header {
  background: color-mix(in srgb, var(--waft-surface) 88%, transparent);
  backdrop-filter: blur(14px);
}
```

Otherwise the page is opaque. Mobile nav panel is solid `--waft-panel`.
Terminal cards use the theme-aware `--waft-code-bg`.

### Layout rules (fixed elements)

- **Header:** `position: sticky; top: 0; z-index: 40;` with a 1px bottom border and the blurred wash above.
- **Skip link:** off-screen by default, slides in on focus (`transform: translateY(0)`).
- **Mobile nav:** drops below header as a contained panel, not a full-screen overlay. Max-height clamped to `min(70vh, 420px)`, scroll-y.

### Color usage in imagery

There is no imagery. If product/marketing imagery is ever added, target: **cool-warm**, low-saturation, with the teal as the dominant non-neutral. Avoid blue, purple, and pink entirely.

### Dark mode

Built in via `data-theme` on `<html>` (light / dark / auto), falling back to `prefers-color-scheme`. The dark palette pushes the surface into a faint green-black (`#101512`) and keeps accents legible by lightening teal to `#8fd7cc`. Unlike earlier versions, the theme is now **uniform**: terminal/code surfaces (`--waft-code-bg`), the footer, and the on-page brand mark all follow the active theme rather than staying dark. The favicon and `waft-mark.svg` keep their dark chip for browser chrome and other fixed contexts; the wordmark assets are chip-less.

---

## Iconography

**The waft codebase ships with exactly one icon: the brand mark.** There is no icon font, no Lucide/Heroicons import, no SVG sprite, no Material Symbols. The CSS uses `currentColor`-stroked SVG only for inline UI atoms (a couple of chevrons / hamburger menus declared in landing CSS rules like `svg { stroke: currentColor; stroke-width: 2; }`), and the favicon.

So the rule is: **don't add icons.** When you reach for one, ask whether the design actually needs it. The docs site has zero icons in body content and the answer is almost always no.

If you *must* add UI icons (only for genuinely necessary affordances — copy buttons, external-link indicators, hamburger), follow these rules:

- **Stroke-based, not filled.** Match the site's existing inline-SVG rule: `fill: none; stroke: currentColor; stroke-width: 2; stroke-linecap: round; stroke-linejoin: round;`.
- **18×18 inside buttons** (`.button__icon`), **16×16 inside the copy button**.
- **No color of their own.** Always `currentColor`.
- **CDN substitute (flagged):** if you can't hand-roll a clean 2-stroke icon, use [Lucide](https://lucide.dev) — same stroke weight, same line-cap style. Flag the substitution. **Heroicons (filled) and Material Icons do not fit** — they're either too detailed or too geometric.

**No emoji. Anywhere.** Not in headings, not in callouts, not in error states. The docs are sans-emoji.

**Unicode glyphs?** None used in the source. Don't introduce them.

### The brand mark

Single SVG, two ink colors, on an 80 viewBox:

- Dark rounded square (`#1f2328`, radius 17) in fixed contexts.
- Boxed computer-fan line art: square frame + circular rim filled in `--waft-accent` (`#16a085`), with corner mounting screws.
- Swept pinwheel blades + hub filled in `--waft-accent-2` (`#d6c24a`).

Conceptually: a fan — air in motion, the *waft*.

**On-page vs. fixed.** In UI (site header, previews) the mark renders **chip-less and theme-aware** — just the teal frame/rim (`--waft-accent`) and mustard blades (`--waft-mark-2`), inline so both fills follow the active theme and no dark box appears in light mode.

**Full-color vs. monochrome.** Two mark treatments: the **full-color** fan (teal frame/rim + mustard blades) for use with enough isolation to stay legible, and a **monochrome** fan (every element one color) for tight or high-contrast contexts. The wordmark uses the monochrome fan set to the text color, so the mark never fights the adjacent "waft" or sits on its own chip. `waft-mark.svg` and `favicon.svg` keep the dark rounded chip for fixed contexts; the wordmark assets are chip-less.

Variants in `assets/`:
- `waft-mark.svg` — full-color fan on a dark rounded chip.
- `favicon.svg` — same, used as `<link rel="icon">`.
- `waft-wordmark.svg` — chip-less monochrome fan + "waft" in Inter Black, ink-colored for light bg.
- `waft-wordmark-light.svg` — chip-less monochrome fan + wordmark in `#f8fafc` for dark bg.

See [`ATTRIBUTION.md`](ATTRIBUTION.md) for the source artwork attribution and
license fallback that must accompany redistributed adaptations.

---

## Components

Three exported React components ship in `components/`, reachable from the compiled bundle as `window.WaftDesignSystem_bb1143.<Name>`. Each has a `.d.ts` (types), a `.jsx` (implementation), and a `@dsCard` HTML preview.

- **Button** — `variant`: `primary` | `secondary` | `ghost`. Renders an `<a>` when `href` is set, else a `<button>`. 36px skeleton, hairline border, subtle shadow.
- **Badge** — `tone`: `neutral` (tags) | `accent` (eligible) | `warn` (excluded). Mono, 12px, hairline pill.
- **TerminalCard** — the brand's hero element. Theme-aware mono command block
  with header + Copy button; takes `lines` (string[]), `title`, `meta`,
  `copyText`.

```js
const { Button, Badge, TerminalCard } = window.WaftDesignSystem_bb1143;
```

The UI-kit JSX under `ui_kits/site/` is separate — those are in-browser Babel recreations of whole screens, not bundle exports.

---

## Index

```
.
├── README.md                  ← you are here
├── SKILL.md                   ← Agent Skill metadata + invocation
├── ATTRIBUTION.md             ← source artwork provenance + license fallback
├── styles.css                 ← single entrypoint (@imports colors_and_type.css)
├── colors_and_type.css        ← CSS custom properties + semantic .waft styles
│
├── components/                ← exported React components + @dsCard previews
│   ├── Button.{jsx,d.ts}, button.html
│   ├── Badge.{jsx,d.ts}, badge.html
│   └── TerminalCard.{jsx,d.ts}, terminalcard.html
│
├── assets/
│   ├── waft-mark.svg          ← brand mark
│   ├── waft-wordmark.svg      ← mark + wordmark (light bg)
│   ├── waft-wordmark-light.svg← mark + wordmark (dark bg)
│   └── favicon.svg            ← favicon (same mark)
│
├── preview/                   ← design-system tab cards (one concept each)
│   └── *.html
│
├── ui_kits/
│   └── site/                  ← docs website UI kit
│       ├── README.md
│       ├── index.html         ← interactive landing → doc clickthrough
│       └── *.jsx              ← React components (Header, Hero, TerminalCard, …)
│
└── site/                      ← historical source snapshot; reference only
    ├── package.json
    ├── src/styles/{theme,landing,starlight}.css
    ├── src/content/docs/*.mdx
    └── …
```

### What to use when

- **Building a marketing page or landing variant for waft?** Open `ui_kits/site/index.html`, use its component anatomy as a reference, and port the result into the target stack.
- **Building a docs page?** Port the token mappings into the production
  Starlight bridge in the parent directory; the copy under
  `site/src/styles/starlight.css` is reference-only.
- **Styling an unrelated artifact in the waft brand?** Link `styles.css` (single entrypoint) and use the `--waft-*` variables.
- **Need a button, badge, or terminal block?** Import from the bundle — see **Components** above.
- **Writing copy?** Re-read **Content Fundamentals** above. The voice is opinionated.

---

## Caveats & known gaps

- **No production-licensed font files.** This system uses Google Fonts CDN for Inter + JetBrains Mono to match the live site. If you need offline/self-hosted files, drop them in `fonts/` and swap the `@import`.
- **No icon library.** Intentional. See **Iconography**.
- **No imagery.** Intentional. See **Visual Foundations → Backgrounds**.
- **Astro/Starlight not reproduced.** The UI kit is plain HTML + React; it
  isn't an Astro app. Use the production application in the parent directory
  for builds and deployment.
