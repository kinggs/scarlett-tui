# Design rules

These rules govern how `scarlett-tui` (the Ratatui TUI in `src/ui.rs`) and the
Omarchy bar plugin (`Panel.qml`) show state. Both surfaces follow the active
Omarchy theme and own no palette, so there is no separate design system: this
file is it.

**This file wins over any drawing.** The
[design canvas](https://claude.ai/artifact/VJRtcSt5woExMGJ4kzsXru) holds the
wireframes and the critique; its character counts are approximate. When a
drawing and this file disagree, follow this file and fix the drawing later.
Background: [issue #6](https://github.com/kinggs/scarlett-tui/issues/6).

The canvas is public. Each board has its own link: add
`#artboard-<board>` to the canvas URL, for example
[`#artboard-Tokens`](https://claude.ai/artifact/VJRtcSt5woExMGJ4kzsXru#artboard-Tokens)
(states), `#artboard-TUI-Narrow-IO` (95×30 I/O) or `#artboard-Panel-BarStates`
(bar icon). A pull request links the boards it implements.

## Rulings

Kenny ruled on these on 27 November 2026. Change them only with a new ruling.

| Topic | Ruling |
|---|---|
| TUI size | Design for the 95×30 floating window (`TUI.float`, 875×600 px ≈ 100×30 cells ⚠ estimated). Under 80×24, show a centred "terminal too small" message instead of a broken layout. |
| 48V | Enabling always takes two steps: arm, then confirm within 3 s. Same in the TUI and the panel. No setting turns this off. Disabling is one step. |
| Bar mouse | Left-click opens the panel. Right-click (and middle-click) opens the TUI. Scroll does nothing, so a stray scroll never changes a level. The tooltip says so. |
| Design system | None. Rules live here; the canvas's tokens board is the drawing. |
| Panel scope (default, unruled) | The panel covers monitoring and common controls. The mix matrix and full routing stay in the TUI. |

## Six rules

1. **Text is foreground.** Readable content uses `Color::Reset`. Secondary
   content adds `Modifier::DIM`. Coloured text is only for short status words
   that also carry a glyph.
2. **DarkGray is decoration.** Use it for borders, the empty meter track and
   `·` for −∞, never for words. ANSI bright-black maps to the theme's `muted`,
   measured at 1.9:1 contrast in Tokyo Night and Catppuccin Latte (WCAG asks
   4.5:1).
3. **Alerts are badges.** 48V live, CLIP, MUTED and errors render reversed:
   coloured background, background-coloured text, bold. Red text alone is 4.8:1
   and yellow 2.3:1 in Latte.
4. **Cursor is REVERSED; the current choice is `●`.** Reversed is the only
   focus cue that works in every palette. The current value in a list gets `●`,
   never colour alone.
5. **Device vs software.** Hardware routing uses cyan. PipeWire graph links use
   magenta. Tab 5 never borrows Tab 3's colour.
6. **No Black, no RGB.** Omarchy maps ANSI Black to the background. No hex
   colours in `src/ui.rs`; no `"#` colour literals in `Panel.qml`. In the panel,
   use only `Commons.Color` roles (`foreground`, `background`, `accent`,
   `urgent`, `muted`) and blends of them.

**No state is shown by colour alone.** Every state below pairs its colour with
a glyph, a word, a modifier or a badge.

## States

| State | TUI | Panel | Notes |
|---|---|---|---|
| Normal value | `Reset`; values the user edits are `BOLD` | `Color.foreground` | Default for everything readable. |
| Secondary | `Reset + DIM` | `Util.alpha(foreground, 0.66)` | Replaces DarkGray text everywhere. |
| Cursor / focus | `REVERSED` row; focused pane title `BOLD` with a cyan border | `CursorSurface` `hasCursor` | |
| Current choice | `●` prefix, `BOLD`; others `○` DIM | `Button` `selected` | Route picker, enum chips. |
| On / active | Green word + `■` | `selected` with accent | Off is DIM `□`, not DarkGray. |
| Caution | Yellow `BOLD` + `▲`; badge form `bg Yellow` | `Qt.tint(accent, alpha(urgent, 0.5))` | Unlocked sync, armed 48V, stale picker. |
| 48V live | Badge ` 48V `: `bg Red`, fg `Reset`-bg, `BOLD`; plus a header badge while any channel is powered | Urgent-filled chip; `Style.selectedStateColor(fg, accent, urgent)`; dot on the bar icon | Two steps to enable, one to disable. |
| 48V armed | Badge ` ARM 48V? ` (`bg Yellow`) and a countdown | "Click again to enable 48V" for 3 s | Esc or timeout cancels. |
| Clipping (latched) | Badge `CLIP` (`bg Red`) until `C` | Urgent dot on the meter row and a ring on the bar icon | Panel clears on click. |
| Muted output | Badge `MUTED` (`bg Red`); level bar becomes DarkGray | `MUTE` button filled urgent; live is a quiet outline | Label is always `MUTE`. |
| Read-only | `DIM` value + `⊘`; "read-only" hint on focus | `enabled: false`, opacity 0.45, tooltip "Set on device" | Replaces `[RO]`. No emoji. |
| Unknown value | `?` in Yellow `BOLD` | `—` with a tooltip | Control exists, value unreadable. |
| Mixer unavailable | Yellow `▲` header; body shows fix steps | Icon dim + `!` badge; panel shows fix steps | Device present, no controls. |
| Disconnected | Red `✕` header; centred empty state | Slashed glyph `󰓄`, opacity 0.45 | Distinct from backend down (panel only). |
| PipeWire link | Magenta type tag; loopback `⟲ loopback` | Count in the footer status line | Software, not stored on the interface. |

## Glyphs

| Glyph | Meaning | ASCII fallback (`--ascii`) |
|---|---|---|
| `█` | meter fill | `#` |
| `░` | empty meter track (DarkGray) | `-` |
| `│` | peak-hold tick; column rule | `\|` |
| `●` / `○` | current / other choice; sync locked | `*` / `o` |
| `■` / `□` | on / off | `[x]` / `[ ]` |
| `▲` | caution | `!` |
| `✕` | disconnected or error | `x` |
| `⊘` | read-only | `RO` |
| `·` | −∞ in the matrix and meters | `.` |
| `⟲` | PipeWire loopback | `~` |
| `◂ N` / `N ▸` | N columns hidden left / right | `< N` / `N >` |
| `↓ N more` | N rows hidden below | `v N more` |

## Meter zones

Both surfaces use the same thresholds.

| Zone | Range | TUI | Panel |
|---|---|---|---|
| Signal | below −9 dBFS | Green | `Commons.Color.accent` |
| Hot | −9 to −1 dBFS | Yellow | `Qt.tint(accent, alpha(urgent, 0.5))` |
| Clip | −1 dBFS and above | Red, and latch `CLIP` | `Commons.Color.urgent`, and latch the clip dot |

- The peak-hold tick `│` holds for 1.5 s, then falls.
- A clip latches until cleared (`C` in the TUI, click in the panel).
- The scale row reads −48 −24 −12 −6 −3 0.
- ⚠ Focusrite's Gain Halo scale runs −42 to 0 dBFS; its zone boundaries are
  not confirmed, so aligning to it is a follow-up.

## Level-jump guard

`0` sets a level to unity (0 dB). A jump of more than +6 dB needs a second `0`
within 2 s; the footer states the jump ("Press 0 again: +34 dB to 0.0 dB").
Downward jumps are instant. This applies to input gain, output volume and
matrix cells.

## Keymap

One keymap table drives the event loop, the footer hints and the `?` overlay,
so they cannot drift. The footer has two rows: transient status and selection
context on row 1; the active tab's highest-value hints on row 2, ending in
`? help`. Block titles hold names only, never hints.

**Everywhere**

| Key | Action |
|---|---|
| `1`–`5` | Jump to tab |
| `Tab` / `Shift-Tab` | Next / previous tab |
| `r` | Rescan device |
| `c` | Switch card (2+ cards) |
| `?` | Help overlay |
| `q`, `Ctrl-C` | Quit |

**Safety**

| Key | Action |
|---|---|
| `Space`, `Space` | Enable 48V (arm, then confirm within 3 s) |
| `C` | Clear clip latches |
| `Esc` | Cancel an armed action; close an overlay |

**I/O tab**

| Key | Action |
|---|---|
| `j` / `k`, arrows | Move |
| `h` / `l`, `-` / `+` | ±1 dB |
| `H` / `L` | ±6 dB |
| `0` | Unity (confirm if > +6 dB) |
| `m` | Mute output |
| `Space`, `Enter` | Toggle or expand |

**Matrix tab**

| Key | Action |
|---|---|
| `h` `j` `k` `l` | Move cell |
| `+` / `-` | ±1 dB |
| `{` / `}` | ±6 dB |
| `0` | Unity (confirm if > +6 dB) |
| `m` | Cell off (−∞) |
| `PgUp` / `PgDn` | Page rows |
| `Home` / `End` | First / last mix |

**Route picker**

| Key | Action |
|---|---|
| `/` | Filter sources |
| `Enter` | Set |
| `Esc` | Cancel |

## Layout

- **Tabs:** `1 Overview`, `2 I/O`, `3 Device routing`, `4 Matrix`,
  `5 PipeWire graph`. Tab 5's subtitle: "software links, not stored on the
  interface".
- **Header priority**, left to right, kept as width shrinks: sync `●` · sample
  rate · 48V badge · clip badge · model. USB ID and firmware drop out below
  120 columns and move to the help overlay's About block.
- **Overview:** under 120 columns, meters stack full-width above the summary.
  With 20+ channels, groups collapse to one line each with `↓ N more`.
- **Panel:** 400 px wide. Model name on line one; status pills on line two
  (`● Locked · 48 kHz · Gen 3`); firmware in the tooltip. One `TUI` button;
  the footer is a status line.

## Colour-free modes

- `NO_COLOR` (any non-empty value), `TERM=dumb` and `--no-color` drop all
  colour and keep modifiers (bold, dim, reversed), glyphs and badges' words.
- `--ascii` swaps the glyphs for the fallbacks in the table above.
