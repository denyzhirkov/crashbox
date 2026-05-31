# Crashbox UI — design notes

The implemented UI follows the **"Signal / aurora"** direction: premium minimalism,
dark-only, with a single violet→cyan accent used sparingly as a *signal* rather than a
theme. The goal is a personal error inbox for one developer or a small team — the
deliberate opposite of an enterprise observability platform.

Tokens and the framework-agnostic component styles live in `frontend/src/index.css`
(translated into Tailwind v4 `@theme` + CSS custom properties). Visual primitives are in
`frontend/src/components/primitives.tsx`.

## Aesthetic direction

- **Base:** near-black, faintly warm neutral ramp (oklch, hue ~73, chroma ~0.006). Not
  pure black, not VS-Code blue-grey. Lots of negative space — airy, not dense.
- **One accent — violet→cyan gradient.** Used only for: keyboard focus, the single primary
  action per screen, and the "live / unresolved" signal. Never a full background wash,
  never on large surfaces.
- **Severity is its own muted set** (fatal / error / warning / info / debug), kept distinct
  from the accent so the two never compete.
- **Glass / depth, subtle.** Frosted low-opacity card surfaces, 1px hairline borders, a
  barely-there accent glow on the focused element. Light and borders define structure —
  heavy drop-shadows are wrong here.
- A single faint aurora bloom sits behind the app (`body` radial gradients), fixed-attach.

## Typography

- **UI text & headings:** Geist Sans (`@fontsource-variable/geist`, self-hosted).
- **All machine data is mono:** issue titles, exception types & messages, stack frames,
  file:line, DSNs, event ids, raw JSON, timestamps. This content *is* code — JetBrains Mono.
- Tabular figures (`tnum`) for all counts.

## Layout primitives

- **One column, no sidebar.** Slim sticky top bar (wordmark + current project chip + ⌘K
  search button + user email + logout) over a centered ~1180px content column. A persistent
  fixed footer hint strip shows the active shortcuts (`j/k` nav · `/` search · `⌘K` commands
  · `↵` open) Linear/Superhuman-style, with a pulsing "live" dot.
- **Issue list as a tape, not a table.** Each row reads left→right: severity edge cue,
  tabular event count (`1,204×`), 24h sparkline, mono title (truncates), relative last-seen
  pushed right. No column headers. The keyboard cursor row gets the accent glow + a gradient
  left bar; resolved issues are dimmed ~50% (never badged, never green).
- **Issue detail: vertical document, not tabs.** Header (glow severity cue, mono title, meta
  strip, snooze dropdown + mark-fixed/reopen) → inline event scrubber → collapsible sections:
  exception → breadcrumbs → tags → user → request → raw JSON. Each section is a card with a
  `[−]`/`[+]` toggle.
- **Stack trace: card stack of frames.** In-app frames weighted, vendor frames dimmed, the
  top frame marked `TOP` and auto-expanded with its source snippet (built from
  pre/context/post lines, the offending line highlighted in the error color).

## Signature elements

1. **24h sparkline** inline in every issue row — a tiny frequency receipt, one hand-rolled
   SVG, recent bars weighted, accent gradient available.
2. **Severity edge cue** (a 3px bar; `dot` and `glow` variants exist) instead of pill/badge soup.
3. **Inline event scrubber** — a strip of ticks, oldest→latest, active tick in the accent
   gradient; click to time-travel that event's data into the view below. No paginated tab.
4. **DSN click-to-copy** — one mono block; the copied affordance is a brief underline, not a toast.
5. **Command palette (`⌘K`)** as the primary navigation. Floating panel over a dimmed
   backdrop, grouped (this issue / this project / navigate / session), arrow-key cursor with
   the accent on the active row, context-aware commands, footer hints. Opened by ⌘K, the
   top-bar search button, or the project chip.
6. **Voiced empty / error states**, mono, lowercase:
   - issues, none: `// nothing's on fire`
   - search, no results: `// 0 matches. broaden the query or check the filter.`
   - new project, no events: `// waiting for the first crash`
7. **Keyboard-first focus** — visible focus uses the accent gradient ring/glow, never a
   default blue outline.

## Microcopy

Lowercase, terse, code-flavored.

- Login button: `unlock`
- Resolve: `mark fixed` · un-resolve: `reopen` · snooze menu: `1h / 1d / 1w / until next crash / wake now`
- Rotate key confirm: `// this invalidates the current DSN. SDKs using the old key will get 401.`
- Unified search/filter: `level:error env:production release:1.4.2` — typed filter keys map to
  dedicated backend params and render as removable chips; clicking a tag on issue detail adds
  it as a filter. Searching and filtering are one input.
- Timestamps: relative by default (`12m ago`), absolute on hover.

## Theme: dark only

Dark-only by decision — no light mode, no theme customizer, the accent is fixed. Hover/focus
is a subtle background shift; the focused row's accent ring is the load-bearing focus signal.

## What we are NOT building

- No charts page — sparklines per row are the only data-viz.
- No query builder — just the unified search/filter input.
- No avatars / mentions / activity feeds; no theme toggle.

## North-star one-liner

> A small, precise instrument for the crashes you actually own — signal, not a dashboard.
