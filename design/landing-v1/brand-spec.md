# Hold Engine — Brand Spec (v1 proposal)

> Status: **PROPOSAL — pending user approval**
> Date: 2026-05-02
> Authoring rationale: no existing brand assets. Charter and ADR voice
> are the only "context" available; this spec proposes a visual identity
> that fits that voice. User can accept, reject, or remix before any
> landing-page polish lands.

## Audience & positioning

- **Audience:** developers tired of Unity/Unreal — paid royalties, fought
  hot-reload, ate runtime bloat, watched their editor crash and lose work.
  Comfortable in the editor, may or may not write code. Want to ship.
- **Pitch:** *"An open-source game engine for people who got tired of
  paying royalties to ship their work. Visual gameplay nodes, real Rust
  escape hatch, hot-reload that doesn't lie, deterministic by default.
  MIT forever."*
- **Anti-audience:** AAA studios with Unreal site licenses, Unity hobbyists
  who only want to make tiny prototypes, anyone looking for visual
  scripting that can't drop into real code.

## Voice keywords (pulled from the Charter + ADRs)

The Charter and ADR 0003 already define a strong voice — the brand spec
is just making it visible:

- **Confident** — "best open-source engine in the world" (Charter)
- **Honest** — "hot-reload that doesn't lie", "saving never loses work"
- **Anti-corporate** — no royalties, no subscriptions, no required online services
- **Tastefully over-engineered** — pays the right tax in the right places
- **Technical** — owns the full stack OS-to-gameplay
- **Patient** — "best engine in the world *someday*", not next week

Translates to: serif typography (not corporate sans), warm paper tones
(not cold blue/white SaaS), one declarative accent (not multi-color
gradient), real code samples (not buzzword bullets), real ADR citations.

## Engine name

**HOLD** — used as both the project name (per `README.md`) and the
display wordmark on the landing page.

Etymology overlap (intentional): "hold the line" against the trends the
Charter calls out (royalty creep, render-pipeline fragmentation, brittle
hot-reload), plus a craftsman's "hold" — grip, control, ownership.

Variants:
- `HOLD` — wordmark (display, all caps, Fraunces opsz=144 wght=600)
- `Hold Engine` — full title for body copy (Fraunces opsz=18 wght=500)
- No icon mark in v1 — wordmark only. Icons get added when they earn
  their place; we don't fake an "engine logo" with a triangle and a
  swoosh.

## Color palette

Single accent. Warm paper-and-ink base. Anti-corporate, anti-SaaS.

| Role | Hex | Notes |
|------|-----|-------|
| Paper (background) | `#FAF7F2` | Warm off-white — not pure `#FFFFFF`. Has weight on screen. |
| Ink (primary text) | `#1A1612` | Deep brown-black, not pure black. Reads as printed, not LCD. |
| Rust (accent) | `#B8431A` | Single signature color. Confident, anti-corporate, evokes oxidized metal / printer ink / Carmack-era industrial. Used for: links, key numbers, the hero CTA, the wordmark. |
| Forest (signal) | `#2D4A2A` | Used sparingly — only for "good" signals in the comparison table (we have hot-reload, Unity does not). |
| Stone (muted text) | `#6B5F52` | Warm gray for secondary copy + captions. Not a cold neutral. |
| Pencil (rules) | `#D9D0C3` | Hairline borders, table dividers. Reads as drafting paper. |

Reference: PostgreSQL docs paper-feel, Berkshire Hathaway Letters PDF
typography, mid-century industrial design manuals. Definitely *not*
Stripe's cold blue or Linear's purple gradients.

**Forbidden colors** (would betray the brand):
- Purple gradient anywhere
- Cold corporate blue (`#0066FF` and friends)
- Pure white background
- Dark-mode-default — the brand is *paper*, not terminal. Dark mode
  will exist as a developer preference, but the brand identity is light.

## Typography

Single-family display + body (Fraunces) plus monospace for code. Avoids
Inter/Roboto/SF Pro slop entirely.

### Fraunces (Google Fonts, free, variable)

Variable axes used: `opsz` (optical size 9–144), `wght` (100–900),
`SOFT` (0–100), `WONK` (0–1).

| Use | Settings |
|-----|----------|
| Display headline (H1) | Fraunces · `opsz: 144` · `wght: 600` · `SOFT: 50` |
| Section heading (H2) | Fraunces · `opsz: 96` · `wght: 500` |
| Subhead (H3) | Fraunces · `opsz: 36` · `wght: 500` |
| Body | Fraunces · `opsz: 18` · `wght: 400` · line-height 1.55 |
| Caption / footnote | Fraunces · `opsz: 14` · `wght: 400` · color `Stone` |

Why Fraunces specifically: **distinctive without being precious**.
Variable optical-size axis means body and display read as the same
voice but optimized for their context. Free on Google Fonts. Looks
nothing like Inter or Söhne or any current SaaS landing page.

### JetBrains Mono (Google Fonts, free)

Code samples + version numbers + ADR citations.

`wght: 500` for code blocks. `wght: 400` for inline code. Monospace
ligatures enabled for `->`, `=>`, `!=`, etc. — developers expect them.

**Forbidden type pairings:**
- Inter / SF Pro / Roboto / Helvetica / Arial as display
- Any rounded sans (Nunito, DM Sans without care)
- Multiple display families (one is enough; two looks like a deck)

## Logo / wordmark usage

In v1 the wordmark *is* the logo:

```
HOLD
```

Set in Fraunces opsz=144, weight 600, letter-spacing -0.02em, color
`Ink` (primary) or `Rust` (accent contexts only). Period included
optionally for a typographic full-stop ("HOLD.") — this is the brand's
"declarative" character punctuation.

A wordmark is enough until the engine has a v1 release worth marking.
A logo without a product feels like a startup wearing a uniform before
finishing the chores.

## Visual signature elements

These are the "120% details" — the places where the brand pays
disproportionate attention to make the rest feel earned:

1. **The wordmark** — every page ends with `HOLD.` set tight, big,
   in `Rust`. Not a button, not a CTA — a sign-off.
2. **ADR citations** — links to ADRs render as `ADR 0004 Q14 →` in
   `JetBrains Mono`, `Stone` color, with the arrow as a real arrow,
   not a `>`. Looks like a footnote in a serious paper.
3. **Comparison table** — uses real symbols (✓ in Forest, ✗ in Ink at
   30% opacity, never red). Reads as a quietly devastating fact sheet,
   not a marketing chart.
4. **Code samples** — set in JetBrains Mono with full Rust syntax
   highlighting in three colors only: Ink for code, Rust for keywords,
   Stone for comments. No rainbow VS Code theme.
5. **The hero "node graph"** — the visual gameplay editor mockup. Drawn
   in CSS with restrained palette, real connection lines that read as
   schematic-diagram, not animated marketing motion.

## Forbidden patterns (anti-AI-slop)

Per the design skill's hard rules:

| Element | Status | Why |
|---------|--------|-----|
| Purple gradient backgrounds | ❌ | AI default for "tech" — strips brand |
| Emoji as bullet icons | ❌ | Reads as Notion template, not editor |
| Rounded cards with left-color border | ❌ | 2020-2024 Material slop |
| AI-painted SVG hero illustrations | ❌ | Will look generic — use real code samples |
| "Get Started Free" CTA buttons | ❌ | The whole engine is free; saying so is condescending |
| Dark mode as the brand identity | ❌ | Paper is the brand; dark mode is a preference |
| "Trusted by" logo strip | ❌ | We have no users yet; pretending otherwise is a lie |
| Rotating hero text / parallax / scrolljacking | ❌ | Substance over showmanship |

## Required content elements (anti-data-slop)

| Element | Required | Why |
|---------|----------|-----|
| Real Rust code in the hero | ✅ | Compiles, isn't pseudo-code, proves substance |
| Real ADR citations as links | ✅ | "ADR 0004 Q14" reads as receipts, not buzzwords |
| Real comparison numbers (binary size, hot-reload time) | ✅ | "<10MB" with the actual measured value, not "blazing fast" |
| Real screenshot of the editor (when one exists) | ✅ | C-direction blend; greyboxed in Junior pass |
| MIT license badge or text | ✅ | Trust signal that costs nothing |
| Link to GitHub repository | ✅ | Code is the proof |

## Forbidden content elements

| Element | Status | Why |
|---------|--------|-----|
| Fake testimonials | ❌ | We have no users; lying is malpractice |
| Vague metrics ("10x faster", "blazing fast") | ❌ | Confident voice is honest, not hyperbolic |
| "Limited time" anything | ❌ | MIT forever is the deal |
| Email-capture / newsletter modal | ❌ | We respect your time |
| Cookie banner | ❌ | Static page, no cookies, no banner needed |

---

## Open questions for the user

1. **Wordmark style** — accept "HOLD" as plain Fraunces wordmark, or
   want me to explore a custom-cut variant (sharper terminals,
   stencil cuts, etc.)?
2. **Color** — the accent is Rust `#B8431A`. Alternatives I considered:
   - Brass `#A37B22` (warmer, more "drafting room")
   - Oxblood `#7A1F1F` (more anti-corporate, possibly aggressive)
   - Forest `#2D4A2A` (calmer, but already used as the "signal" color)
3. **Typography** — Fraunces variable for both display + body, or split
   to a different body face (Newsreader / Source Serif)?
4. **Charter conflict** — the visual-scripting story flips ADR 0003's
   "non-goal: visual scripting." Does that get a charter amendment, a
   new ADR (0006), or do we treat the landing page as forward-projection?

These don't need to be answered before I build the Junior pass — I'll
proceed with the defaults above, and the page itself becomes the
prompt for your reaction.
