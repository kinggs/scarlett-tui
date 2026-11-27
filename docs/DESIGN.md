# Design rules

These rules govern how `scarlett-tui` (the Ratatui TUI in `src/ui.rs`) and the
Omarchy bar panel (`Panel.qml`) show state. Neither surface owns a palette, so
there is no separate design system: this file is it.

- **The TUI runs in any terminal on any Linux.** It uses only the 16 ANSI
  colours, so it follows whatever theme the terminal has. Where this file
  quotes a contrast figure or a theme name, that is where it was measured
  (Omarchy's Tokyo Night and Catppuccin Latte terminal themes), not a
  requirement.
- **The panel runs only in the Omarchy shell.** It follows the Omarchy theme
  through `Commons.Color`. The "Panel" columns and sections below apply only
  there.

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
| TUI size | Design for a 95×30 terminal. Omarchy's floating TUI window (`TUI.float`, 875×600 px) holds more: measured at 113×35 cells in foot with Omarchy's default JetBrainsMono Nerd Font 9 pt, on a 1.6×-scaled display (the first estimate, ≈100×30, was low). 95×30 stays the target: it fits there with room, and is a sensible small window anywhere. Under 80×24, show a centred "terminal too small" message instead of a broken layout. |
| 48V | Enabling always takes two steps: arm, then confirm within 3 s. Same in the TUI and the panel. No setting turns this off. Disabling is one step. |
| Bar mouse | Left-click opens the panel. Right-click (and middle-click) opens the TUI. Scroll does nothing, so a stray scroll never changes a level. The tooltip says so. |
| Design system | None. Rules live here; the canvas's tokens board is the drawing. |
| Panel scope (default, unruled) | The panel covers monitoring and common controls. The mix matrix and full routing stay in the TUI. |

Kenny ruled on these later on 27 November 2026, after the UX, simplicity and
keys pass. Boards:
[keys](https://claude.ai/artifact/VJRtcSt5woExMGJ4kzsXru#artboard-Proposed-Keys),
[I/O board](https://claude.ai/artifact/VJRtcSt5woExMGJ4kzsXru#artboard-Proposed-IO-Board),
[Overview](https://claude.ai/artifact/VJRtcSt5woExMGJ4kzsXru#artboard-Proposed-Overview-Meters),
[panel](https://claude.ai/artifact/VJRtcSt5woExMGJ4kzsXru#artboard-Proposed-Panel-Merged).

| Topic | Ruling |
|---|---|
| Level keys | `-` / `+` are ±1 dB and `{` / `}` are ±6 dB on every tab. `h` `j` `k` `l` and the arrows only move; they never change a value. `H` / `L` are gone. |
| Mute and off | `m` is the only mute (an output on I/O, a cell on the Matrix). `x` is the only disconnect (route off, PipeWire link). `0` is only unity. Space and Enter no longer mute a matrix cell; `d` on the PipeWire tab is gone. |
| 48V keys | Only Space arms and confirms 48V. Enter, `+` and `l` do neither. |
| Overview | A read-only meters tab. The "Status & Signal Path" summary is gone. No key on this tab writes to the device. |
| Panel layout | One block per channel: name, meter, level and its controls. No separate LEVELS section. `C` clears clip latches. |

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
6. **No Black, no RGB.** Themes may map ANSI Black to the background, as
   Omarchy's do, which makes Black text invisible. No hex
   colours in `src/ui.rs` (a unit test checks); no `"#` colour literals in
   `Panel.qml`. The README screenshots need a concrete theme, so their
   painter lives apart, in the test-only `src/screenshot_svg.rs`. In the panel,
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
| Read-only | `DIM` value + `⊘`; "read-only" hint on focus | `enabled: false`, opacity 0.45, tooltip "Set on device" | Replaces `[RO]`. No emoji. A disabled control gets no hover, so the panel lays a hover-only `Hint` over it for the tooltip. |
| Unknown value | `?` in Yellow `BOLD` | `—` in the control (`48V —`, `MUTE —`, `INST —`) and a "could not be read" tooltip | Control exists, value unreadable. |
| Mixer unavailable | Yellow `▲` header; body shows fix steps | Icon dim + `!` badge; panel shows fix steps | Device present, no controls. |
| Disconnected | Red `✕` header; centred empty state | Slashed glyph `󰓄`, opacity 0.45 | Distinct from backend down (panel only). |
| PipeWire link | Magenta type tag; loopback `⟲ loopback` | Not shown; the PipeWire graph is in the TUI | Software, not stored on the interface. |

## Glyphs

| Glyph | Meaning | ASCII fallback (`--ascii`) |
|---|---|---|
| `█` | meter fill | `#` |
| `░` | empty meter track (DarkGray) | `-` |
| `│` | peak-hold tick; column rule | `\|` |
| `●` / `○` | current / other choice; sync locked | `*` / `o` |
| `■` / `□` | on / off | `x` / `.` (a cell holds one character) |
| `▲` | caution | `!` |
| `✕` | disconnected or error | `x` |
| `⊘` | read-only | `/` |
| `·` | −∞ in the matrix and meters | `.` |
| `⟲` | PipeWire loopback | `~` |
| `◂ N` / `N ▸` | N columns hidden left / right | `< N` / `N >` |
| `↓ N more` | N rows hidden below | `v N more` |
| `─` `┌` `┐` … | box drawing | `-` and `+` |
| `−` `—` | minus, unknown | `-` |
| anything else | | `?` |

`--ascii` replaces one cell with one character, so every fallback is a
single character; `ascii_for` in `src/ui.rs` is the full list.

## Meter zones

Both surfaces use the same thresholds.

| Zone | Range | TUI | Panel |
|---|---|---|---|
| Signal | below −6 dBFS | Green | `Commons.Color.accent` |
| Hot | −6 to −1 dBFS | Yellow | `Qt.tint(accent, alpha(urgent, 0.5))` |
| Clip | −1 dBFS and above | Red, and latch `CLIP` | `Commons.Color.urgent`, and latch the clip dot |

- The peak-hold tick `│` holds for 1.5 s, then falls.
- A clip latches until cleared (`C` in the TUI, click in the panel).
- The scale row reads −48 −24 −12 −6 −3 0.
- **Why these numbers.** Hot starts at −6 dBFS, where Focusrite's 3rd Gen
  Gain Halo turns amber
  ([Scarlett 2i2 3rd Gen user guide](https://fael-downloads-prod.focusrite.com/customer/prod/downloads/Scarlett%202i2%203rd%20Gen%20User%20Guide%20V2.pdf):
  green from −24, amber from −6, red at 0 dBFS). The halo goes red only at
  0 dBFS; the clip latch fires at −1 dBFS on purpose, as an early warning.
  Below −24 the halo is dark, but a meter still shows the level, so there is
  no separate zone for it.
- ⚠ The 4th Gen guides draw a halo scale from −42 to 0 dBFS (steps −42 −36
  −24 −18 −12 −6 0) but give no colour boundaries, so the 3rd Gen values
  stand for every model.
- ⚠ With the panel closed, the daemon reads the meters every 200 ms to latch
  clips. The kernel driver asks the device afresh on each read and holds no
  peak; nothing public says whether the device's firmware holds a peak
  between reads. If it does not, a clip shorter than 200 ms can be missed
  until the panel opens (it reads at 20 Hz while open). Not tested on
  hardware.

## Level-jump guard

`0` sets a level to unity (0 dB). A jump of more than +6 dB needs a second `0`
within 2 s; the footer states the jump ("Press 0 again: +34 dB to 0.0 dB").
Downward jumps are instant. This applies to input gain, output volume and
matrix cells.

`{` / `}` move 6 dB per press, so they never need the confirm. `m` on a muted
matrix cell restores the level it had before, without a confirm, even when
that is a rise of more than +6 dB ⚠ (left unguarded on purpose: the level was
audible a moment earlier).

## Keymap

One keymap table drives the event loop, the footer hints and the `?` overlay,
so they cannot drift. The footer has two rows: transient status and selection
context on row 1; the active tab's hints on row 2, ending in `? help` and
`q quit`. Tab, rescan and card keys are in the overlay only: the tab bar
already shows the numbers. Block titles hold names only, never hints.

**One vocabulary.** A key means one thing on every tab. The three families
never overlap: move, change a level, switch a state. A key with no meaning on
a tab does nothing there.

| Key | Action | I/O | Device routing | Matrix | PipeWire graph |
|---|---|---|---|---|---|
| `h` `j` `k` `l`, arrows | Move. Never writes. | row, and control within a row | row | cell | row |
| `-` / `+` | −/+ 1 dB, or previous / next choice | gain, volume, enum | previous / next source | cell | — |
| `{` / `}` | −/+ 6 dB | gain, volume | — | cell | — |
| `0` | Unity (confirm if > +6 dB) | gain, volume | — | cell | — |
| `m` | Mute; again restores | the row's output | — | cell to −∞ | — |
| `Space` | Toggle a switch | Air, Pad, mute, 48V (twice) | — | — | — |
| `Enter` | Open or commit | same as Space, but never 48V | open the source picker | — | — |
| `x` | Disconnect | — | route off | — | disconnect link (`Del` too) |
| `a` | All links / Scarlett only | — | — | — | toggle |

`=` is a silent alias for `+`, and `[` / `]` for `-` / `+`, so Shift makes
the same key bigger. The Matrix also has `PgUp` / `PgDn` (page rows) and
`Home` / `End` (first / last mix).

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
| `Space`, `Space` | Enable 48V (arm, then confirm within 3 s). No other key arms or confirms. |
| `0`, `0` | Unity when the rise is over +6 dB (confirm within 2 s) |
| `C` | Clear clip latches (any tab, and the panel) |
| `Esc` | Cancel an armed action; close an overlay |

Any other key also cancels an armed action, then acts normally.

**Route picker**

| Key | Action |
|---|---|
| `j` / `k` | Move |
| `/` | Filter sources |
| `Enter` | Set |
| `Esc` | Cancel |

**Panel**

| Key | Action |
|---|---|
| Arrows | Move |
| `Space` | Toggle a chip; 48V takes two |
| `Enter` | Open an output's sources; set |
| `m` | Mute the output under the cursor |
| `C` | Clear clip latches |
| `r` / `T` | Refresh / open the TUI |
| `?` | Show or hide the key list in the footer |
| `Esc` | Collapse, then close |

`h` `j` `k` `l` also move. ⚠ The shell's key catcher reports Enter and Space
as one "activate", so in the panel Enter also toggles and arms 48V.

## Layout

- **Tabs:** `1 Overview`, `2 I/O`, `3 Device routing`, `4 Matrix`,
  `5 PipeWire graph`. Tab 5's subtitle: "software links, not stored on the
  interface".
- **Chrome:** one header line, one tab line, the body, then the two footer
  rows. The header has no box; the body's own border is the rule. At 95×30
  that leaves 26 rows for the body.
- **Header priority**, left to right, kept as width shrinks: sync `●` · sample
  rate · 48V badge · clip badge · model. USB ID and firmware drop out below
  120 columns and move to the help overlay's About block.
- **Overview:** meters only, in groups (outputs, inputs, mixer inputs). A row
  ends with the state that explains its meter: `CLIP`, then for a row fed by
  an analogue input that input's ` 48V ` badge, `Inst` and `Air` (words, in
  fixed slots), then `MUTED` or `← source`. "Analogue N" in the source names
  the input; the panel finds an input's meter the same way
  (`ScarlettModel::input_meter`). With more channels than rows, the last line reads
  `↓ N more`.
- **I/O:** a board, not a table. Sections: Inputs, Phantom power, Outputs,
  Device. One row per channel with its controls across; `h` / `l` step between
  them. An output's mute sits on its volume row. 48V has its own section.
  Read-only controls show `⊘`.
- **Matrix:** the legend (`·` off, unity, boost, unrouted mix) is in the help
  overlay, not on the tab.
- **Panel:** 400 px wide, up to 760 px tall (the shell also stops it at the screen edge; longer content scrolls). Model name on line one; status pills on line two
  (`● Locked · 48 kHz · Gen 3`); firmware in the tooltip. One `TUI` button.
  Sections: INPUTS (meter, then chips, then the 48V row) and OUTPUTS (meter,
  level, `MUTE`, then source). The alert strip is for clips only. The footer
  is the key line, and `?` swaps it for the key list. When a model's meters
  cannot be matched to its outputs by name, a separate LEVELS list is shown
  instead.

## Colour-free modes

- `NO_COLOR` (any non-empty value), `TERM=dumb` and `--no-color` drop all
  colour and keep modifiers (bold, dim, reversed), glyphs and badges' words.
- `--ascii` swaps the glyphs for the fallbacks in the table above.
