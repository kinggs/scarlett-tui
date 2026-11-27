//! Paints a rendered TUI buffer as an SVG terminal, for the README
//! screenshots only (`make screenshots`, `ui::tests::demo_screenshots_are_current`).
//!
//! This is the one place with RGB colours. It lives outside `src/ui.rs` so
//! that file keeps DESIGN rule 6 ("no hex colours in `src/ui.rs`") as written:
//! the TUI draws only ANSI colours, and the picture needs a concrete theme.
//! Test-only: compiled under `cfg(test)` and never part of the binary.

use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};

/// Tokyo Night, as Omarchy ships it (`themes/tokyo-night/colors.toml`), for
/// README screenshots only. The TUI itself never uses RGB.
fn svg_color(c: Color, fg: bool) -> &'static str {
    match c {
        Color::Reset => {
            if fg {
                "#a9b1d6"
            } else {
                "#1a1b26"
            }
        }
        Color::Red | Color::LightRed => "#f7768e",
        Color::Green | Color::LightGreen => "#9ece6a",
        Color::Yellow | Color::LightYellow => "#e0af68",
        Color::Blue | Color::LightBlue => "#7aa2f7",
        Color::Magenta | Color::LightMagenta => "#ad8ee6",
        Color::Cyan | Color::LightCyan => "#449dab",
        Color::DarkGray => "#414868",
        Color::Gray | Color::White => "#c0caf5",
        _ => "#a9b1d6",
    }
}

/// Paint a rendered buffer as an SVG terminal: one text run per style span,
/// stretched to the cell grid so columns line up.
pub(crate) fn buffer_to_svg(buf: &Buffer) -> String {
    let (cw, ch, fs) = (8.4_f32, 18.0_f32, 14.0_f32);
    let (w, h) = (buf.area.width, buf.area.height);
    let pad = 12.0;
    let mut out = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\">\n\
         <rect width=\"100%\" height=\"100%\" rx=\"8\" fill=\"#1a1b26\"/>\n\
         <g font-family=\"JetBrainsMono Nerd Font, monospace\" font-size=\"{fs}\">\n",
        w as f32 * cw + 2.0 * pad,
        h as f32 * ch + 2.0 * pad
    );
    let esc = |t: &str| {
        t.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    for y in 0..h {
        let mut x = 0;
        while x < w {
            let c0 = &buf[(x, y)];
            let key = (c0.fg, c0.bg, c0.modifier);
            let start = x;
            let mut text = String::new();
            while x < w && {
                let c = &buf[(x, y)];
                (c.fg, c.bg, c.modifier) == key
            } {
                text.push_str(buf[(x, y)].symbol());
                x += 1;
            }
            let n = (x - start) as f32;
            let rev = key.2.contains(Modifier::REVERSED);
            let (fg, bg) = if rev {
                (svg_color(key.1, false), svg_color(key.0, true))
            } else {
                (svg_color(key.0, true), svg_color(key.1, false))
            };
            let px = pad + start as f32 * cw;
            let py = pad + y as f32 * ch;
            if rev || key.1 != Color::Reset {
                out += &format!(
                    "<rect x=\"{px}\" y=\"{py}\" width=\"{}\" height=\"{ch}\" fill=\"{bg}\"/>\n",
                    n * cw
                );
            }
            if text.trim().is_empty() {
                continue;
            }
            let weight = if key.2.contains(Modifier::BOLD) {
                " font-weight=\"bold\""
            } else {
                ""
            };
            let opacity = if key.2.contains(Modifier::DIM) {
                " fill-opacity=\"0.62\""
            } else {
                ""
            };
            let deco = if key.2.contains(Modifier::UNDERLINED) {
                " text-decoration=\"underline\""
            } else {
                ""
            };
            out += &format!(
                "<text x=\"{px}\" y=\"{}\" fill=\"{fg}\"{weight}{opacity}{deco} textLength=\"{}\" lengthAdjust=\"spacingAndGlyphs\" xml:space=\"preserve\">{}</text>\n",
                py + ch * 0.75,
                n * cw,
                esc(&text)
            );
        }
    }
    out + "</g>\n</svg>\n"
}
