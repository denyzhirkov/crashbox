# Crashbox UI — design notes

Working brief for E7. Not final spec — pick what fits, drop what doesn't survive contact with the actual data. Goal: a small but memorable tool that feels like it was made by one person for developers, not by a committee for a SaaS dashboard.

## Brand positioning

Crashbox is a **box** where crashes land. Small, contained, personal. The opposite of an enterprise observability platform. The visual identity should reinforce this:

- Compact. Dense without being cramped.
- Plain over decorative. No gradients, no glassmorphism, no animated charts.
- Tactile metaphors over abstract dashboards. Think: a printed crash receipt, a developer's bench, an inbox of small physical things — not a "platform".
- Self-aware. The tool tracks errors; when *it* errors, it should look intentional, not generic.

What to avoid: Sentry's purple-gradient look, generic Tailwind UI kits, BootstrapVue-style spacing, dashboard hero numbers, marketing-feel splash on the login page.

## Typography — the single biggest differentiator

Most devtool dashboards default to system sans and look interchangeable. Crashbox leans into **mono-first**:

- **Body & data: mono.** JetBrains Mono, IBM Plex Mono, or Berkeley Mono if available. Self-hosted via `@fontsource/*` so there's no CDN at runtime.
- **Headings & nav: a single contrasty pair.** Options:
  - Mono everywhere (most opinionated). Tabular figures for counts.
  - Mono for data + a single grotesk for nav/headings (Inter, Geist).
  - Mono for data + a low-contrast serif for headings (Newsreader, Source Serif). This is the most "memorable" — feels editorial, almost like a printout, in a category dominated by sans-everywhere.
- Recommended default: **serif headings + mono everywhere else.** It reads "small magazine about your crashes" rather than "dashboard product".

Make exception messages, stack frames, and issue titles all genuinely monospaced. They *are* code; render them as code.

## Color

Single accent over a near-monochrome base. No multi-color status systems.

- **Light:** off-white paper (`#f7f5ef` / warm), graphite text (`#1a1a1d`). Avoid pure white — newsprint feel.
- **Dark:** deep ink with warm undertone (`#15141a`, not `#000`). Soft cream text (`#e9e6df`). Should not look like VS Code.
- **Accent:** one hot color, used sparingly — only for actions and unresolved status. Two candidates:
  - **Crash-red** `#e3402d` — direct, on-brand for an error tracker
  - **Hazard-amber** `#f0b400` — warning-tape feel, less aggressive
- **Severity edge bar**, not a tag chip: 2px vertical bar on the left of each issue row encodes level (red=error, amber=warning, neutral=info). No colored backgrounds, no pills. This is the load-bearing visual cue.

Avoid green for "resolved" — too generic. Resolved issues are just **dimmed** (text drops to 50% opacity, edge bar gone). The eye filters them out.

## Layout primitives

- **One column, never a sidebar shell.** Top bar (logo + project switcher + cmd-k) + content. No left rail eating 240px. The UI looks like a focused inbox, not a console.
- **Issue list as a tape, not a table.** Each row is one line at terminal-density, with a vertical edge bar (severity), a tabular event count, a sparkline of the last 24h burst pattern, the exception title, and a relative timestamp. No column headers — the row reads left-to-right naturally.
- **Issue detail: vertical document, not tabs.** Title → meta strip → stack trace → breadcrumbs → tags → request → user → raw JSON. Each section collapsible (`[−]` toggle). Reads like a printout. No tabbed "Overview / Activity / Tags" navigation; that fragments the only thing the user came here for.
- **Stack trace: card stack of frames.** Each frame is a card with file:line + function name in mono, source snippet collapsed by default, expand on click. In-app frames are visually weighted; vendor frames dimmed. Top-of-stack frame is auto-expanded.

## Specific unique elements

These are the things a returning user will tell their friend about:

1. **Sparkline per issue row.** A 24-hour micro-histogram of event counts, rendered as ASCII-ish bars in mono, inline with the row text. ~20px tall, 80px wide. Reads like a frequency receipt next to each issue. Hover for exact counts. Builds with one SVG, no chart library.
2. **Edge bar severity.** As above — no tag soup.
3. **Keyboard-first navigation.** `j`/`k` to move through issues, `e` to resolve, `o` to open, `/` to focus search, `cmd+k` for command palette, `g p` to go to projects, `?` to show shortcuts. Match Linear/Superhuman muscle memory. Persistent footer with current shortcuts (Slack/Linear-style hint strip).
4. **Command palette is the primary nav.** No traditional menus for project switching, filter switching, status filter — all `cmd+k`. The top bar shows just current project name (clickable to open palette).
5. **Inline event scrubber on issue detail.** A horizontal strip above the trace showing every event of this issue as a tick; click a tick to load that event's data into the same view. No paginated "Events" tab. The user stays in one place and time-travels.
6. **Empty states with voice.** No illustrations. Just one mono line:
   - Issues list, no errors: `// nothing's on fire`
   - Project just created, no events: `// waiting for the first crash. configure your DSN:` + DSN block.
   - Search no results: `// 0 matches. broaden the query or check the filter.`
7. **Self-aware error states.** If the UI itself throws, show: `// crashbox crashed. dogfooding it now.` with a copy button for the trace. This is the kind of detail people screenshot.
8. **DSN reveal.** Single big mono block, click-to-copy, with a subtle "copied" affordance (the text briefly underlines instead of a toast). No modal, no "show DSN" toggle for the project owner — DSN is the project's identity, surface it.
9. **Raw JSON viewer.** Monospaced, key-collapsed at depth 2 by default. Click a key to expand. No 3rd-party JSON viewer with its own theme — render with a custom 60-line component so it inherits Crashbox typography exactly.
10. **Theme: warm dark by default.** System-following, but on first visit the user gets warm dark (matches the brand). Toggle in command palette, not a sun/moon icon in the top bar.

## Microcopy

Lowercase, terse, code-flavored. Examples:

- Login button: `unlock` (not "Sign in", not "Log in")
- Resolve action: `mark fixed` (not "Resolve")
- Unresolve: `reopen`
- Rotate key: `rotate` (with a confirm dialog that says `// this invalidates the current DSN`)
- Filter chips: `level:error`, `env:production`, `release:1.0.0` — same syntax as the search box. Click a chip in a row to add it as a filter. Searching and filtering are literally the same input.
- Settings page heading: `// project settings`
- Timestamps: relative by default (`12m ago`), absolute on hover (`2026-05-28T08:21:14Z` in mono).

## Theme: dark only

Decision: Crashbox is dark-only. Light mode was prototyped early and dropped — it didn't feel cohesive with the brand's warm-ink + crash-red identity, and added a maintenance surface we didn't want.

- Dark base: `#15141a` (a hint warm — not VS Code blue-grey, not pure black).
- Borders: 1px, low contrast (`#26252d`). Borders define structure; not shadows.
- Hover/focus: very subtle background shift (`+4% lightness`), and the edge bar of the focused row jumps to full accent color. Keyboard focus must be visible without a generic blue outline — restyle to use the brand accent.

## Density

Aim for ~22px row height on the issue list. Padding everywhere is 8/16/24 (no 12, no 20 — pick a small scale and stick to it). Default font size 14px mono, 13px on tables, 18px on the few headings.

## What we are NOT building in MVP

- No charts page. Sparklines per row only.
- No "discover/explore" query builder. Just the unified search/filter input.
- No theme customizer. Two themes, accent fixed.
- No avatars / mentions / activity feeds.
- No tooltips for things that have a label already.

## Open questions to resolve at implementation time

- Mono choice: JetBrains Mono is the safe default; check Berkeley Mono licensing if a more distinctive look is wanted.
- Serif choice: Newsreader (Google Fonts, free, well-built) vs Source Serif. Try Newsreader first.
- Sparkline implementation: hand-rolled SVG (recommended) vs `uplot` (overkill).
- Command palette: `kbar` is React-only; build a tiny Solid one (~100 LOC) since the keyspace is small.
- Should resolved issues be hidden by default in the main list, or just dimmed? Lean: dimmed (one less filter to think about), with `/` to type `status:unresolved` if you want strict.

## North-star one-liner

> Crashbox should feel like a small zine about your crashes, not a dashboard product.
