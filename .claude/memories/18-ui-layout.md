# UI layout rules: everything flexible (2026-11-18, Vincent's request)

Vincent asked that "every component is flex": no fixed widths that clip or
overflow, paths visible instead of truncated. Audited every page and dialog
at 1440 / 1100 / 800 / 520 / 380 px with `.claude/tests/scripts/t_flex.py`
(run: `SCRATCH=<scratch> FLEX_OUT=<dir> bash .claude/tests/scripts/pw.sh t_flex.py`
against the 7799 scratch daemon; it reports document overflow, elements past
the viewport edge and ellipsized text, and saves a screenshot per page/width).
Final state: 0 unexpected findings in 70 combinations.

## Rules now in `ui/src/styles.scss`
- `.flex` wraps; `box-sizing: border-box` everywhere; `img/pre` max-width 100%.
- Cards/grid cells `min-width: 0`; block titles (`h1-h4`, card titles, `p`,
  `li`, `dd`, `.note/.meta/.muted`) use `overflow-wrap: anywhere`.
  **Never put overflow-wrap:anywhere on inline elements (`b`, `.mono`,
  `code`)** — in a flex row their min-content width collapses to one letter
  per line (that happened to the form-editor labels).
- `.wrap-list` on a mat-nav-list/mat-list: items grow with their text
  (height auto, MDC two-line `::before/::after` spacers removed, margins
  reset) **and `border-radius: 12px`** — M3 nav items are 9999px pills, so a
  300 px tall multi-line item became a circle that clipped its own first
  characters (took a bisect to find). Used on the setup page lists and Jobs.
- Expansion panel headers: height auto, `.mat-content` wraps, title/description
  `min-width: 0` (the Jobs history header had squeezed titles to one letter per
  line because the description was `flex: 0 0 auto; nowrap`).
- Dialogs ≤ 600 px: `calc(100vw - 16px)` wide.
- `dl dd { min-width: 0; overflow-wrap: anywhere }` and grids use
  `minmax(0, 1fr)` for the value column.

## Per page
- Grids: `repeat(auto-fit, minmax(min(100%, Npx), 1fr))` everywhere (a plain
  `minmax(380px, 1fr)` overflows a 380 px phone).
- Rows (`.row` on remote/flake/channels/secrets/git, `.gen` on system): `flex-wrap:
  wrap`, text block `flex: 1 1 160–200px; overflow-wrap: anywhere`, action
  buttons grouped in `.acts { margin-left: auto }`; `.d` lines wrap instead of
  ellipsis.
- Toolbar (workspace): wraps onto a second row (`height: auto`, `justify-content:
  flex-end`), the path has `flex: 1 1 0; max-width: 40vw` so it shrinks before
  anything wraps and is hidden ≤ 1000 px; ≤ 760 px button labels (`.lbl`) hide,
  icon buttons shrink to 40 px, theme + rescan + home move into a `more_vert`
  menu (`.narrow-hide` / `.narrow-only`); brand text hidden ≤ 560 px.
  `.content` max-width 1400 px, padding `clamp(8px, 2vw, 24px)`.
- Overview import map: `.desc` ellipsis + tooltip on wide screens, wraps to
  full width ≤ 760 px (media rule placed **after** the base `.desc` rule, same
  specificity ⇒ order matters).
- Form editor (`node.component`): the ≤ 700 px media block must be the **last**
  rule (the base `.head { flex-wrap: nowrap }` used to override it); inline
  inputs `flex: 1 1 120px; width: auto` so the `{}` toggle stays on the row.
- Setup: `.wrap` 1100 px, path row wraps, both lists `.wrap-list`.
- System: head wraps, Machine select `width: min(260px, 100%)`.
- Target dialog: label "Flake host" (long label was clipped), detail in the hint.
Remaining ellipses are deliberate: toolbar path, overview descriptions on wide
screens (tooltip), form-editor comments on wide screens (tooltip), Monaco's
own internal lines.
