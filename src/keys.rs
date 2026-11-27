//! The keymap. One table drives the event loop, the footer hints and the `?`
//! overlay, so they cannot drift (docs/DESIGN.md "Keymap").

use crossterm::event::KeyCode as K;

/// Where a binding applies. Modal scopes (`Picker`, `Help`) do not fall back
/// to global keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Global,
    /// Documented in the overlay only; handled by the armed-action logic in `ui.rs`.
    Safety,
    Overview,
    Io,
    Routing,
    Matrix,
    PipeWire,
    Picker,
    Help,
}

/// What a key does, independent of which key it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Quit,
    GoTab(usize),
    NextTab,
    PrevTab,
    Rescan,
    SwitchCard,
    Help,
    Cancel,
    Confirm,
    Up,
    Down,
    Left,
    Right,
    Inc,
    Dec,
    IncBig,
    DecBig,
    Unity,
    Mute,
    Toggle,
    Off,
    PageUp,
    PageDown,
    First,
    Last,
    ShowAll,
    Disconnect,
}

/// One line of the help overlay, and optionally one footer hint.
pub struct Binding {
    pub scope: Scope,
    /// Keys as shown to the user, e.g. `h/l`.
    pub label: &'static str,
    /// Overlay description.
    pub help: &'static str,
    /// Short footer word; `None` keeps the binding out of the footer.
    pub hint: Option<&'static str>,
    pub keys: &'static [(K, Action)],
}

use Action as A;

const fn b(
    scope: Scope,
    label: &'static str,
    help: &'static str,
    hint: Option<&'static str>,
    keys: &'static [(K, Action)],
) -> Binding {
    Binding {
        scope,
        label,
        help,
        hint,
        keys,
    }
}

const MOVE_ROWS: &[(K, Action)] = &[
    (K::Char('j'), A::Down),
    (K::Down, A::Down),
    (K::Char('k'), A::Up),
    (K::Up, A::Up),
];

/// Footer hints are taken in table order within a scope, so put the most
/// useful binding first.
pub const KEYMAP: &[Binding] = &[
    // I/O tab
    b(Scope::Io, "j/k", "move", Some("move"), MOVE_ROWS),
    b(
        Scope::Io,
        "h/l",
        "−/+ 1 dB, or previous / next value",
        Some("±1 dB"),
        &[
            (K::Char('h'), A::Dec),
            (K::Left, A::Dec),
            (K::Char('-'), A::Dec),
            (K::Char('l'), A::Inc),
            (K::Right, A::Inc),
            (K::Char('+'), A::Inc),
            (K::Char('='), A::Inc),
        ],
    ),
    b(
        Scope::Io,
        "H/L",
        "−/+ 6 dB",
        Some("±6 dB"),
        &[(K::Char('H'), A::DecBig), (K::Char('L'), A::IncBig)],
    ),
    b(
        Scope::Io,
        "Space",
        "toggle or cycle (48V: press twice)",
        Some("toggle"),
        &[(K::Char(' '), A::Toggle), (K::Enter, A::Toggle)],
    ),
    b(
        Scope::Io,
        "0",
        "unity (confirm if > +6 dB)",
        Some("unity"),
        &[(K::Char('0'), A::Unity)],
    ),
    b(
        Scope::Io,
        "m",
        "mute output, or minimum level",
        Some("mute"),
        &[(K::Char('m'), A::Mute)],
    ),
    // Device routing tab
    b(Scope::Routing, "j/k", "move", Some("move"), MOVE_ROWS),
    b(
        Scope::Routing,
        "Enter",
        "pick source",
        Some("pick"),
        &[(K::Enter, A::Toggle), (K::Char(' '), A::Toggle)],
    ),
    b(
        Scope::Routing,
        "h/l",
        "previous / next source",
        Some("cycle"),
        &[
            (K::Char('h'), A::Left),
            (K::Left, A::Left),
            (K::Char('l'), A::Right),
            (K::Right, A::Right),
        ],
    ),
    b(
        Scope::Routing,
        "x",
        "route off",
        Some("off"),
        &[(K::Char('x'), A::Off), (K::Char('0'), A::Off)],
    ),
    // Matrix tab
    b(
        Scope::Matrix,
        "h j k l",
        "move cell",
        Some("move"),
        &[
            (K::Char('h'), A::Left),
            (K::Left, A::Left),
            (K::Char('j'), A::Down),
            (K::Down, A::Down),
            (K::Char('k'), A::Up),
            (K::Up, A::Up),
            (K::Char('l'), A::Right),
            (K::Right, A::Right),
        ],
    ),
    b(
        Scope::Matrix,
        "+ / -",
        "±1 dB",
        Some("±1 dB"),
        &[
            (K::Char('+'), A::Inc),
            (K::Char('='), A::Inc),
            (K::Char(']'), A::Inc),
            (K::Char('-'), A::Dec),
            (K::Char('['), A::Dec),
        ],
    ),
    b(
        Scope::Matrix,
        "{ / }",
        "±6 dB",
        Some("±6 dB"),
        &[(K::Char('}'), A::IncBig), (K::Char('{'), A::DecBig)],
    ),
    b(
        Scope::Matrix,
        "0",
        "unity (confirm if > +6 dB)",
        Some("unity"),
        &[(K::Char('0'), A::Unity)],
    ),
    b(
        Scope::Matrix,
        "m",
        "cell off (−∞), or restore",
        Some("off"),
        &[
            (K::Char('m'), A::Mute),
            (K::Char('x'), A::Mute),
            (K::Enter, A::Mute),
            (K::Char(' '), A::Mute),
        ],
    ),
    b(
        Scope::Matrix,
        "PgUp PgDn",
        "page rows",
        None,
        &[(K::PageUp, A::PageUp), (K::PageDown, A::PageDown)],
    ),
    b(
        Scope::Matrix,
        "Home End",
        "first / last mix",
        None,
        &[(K::Home, A::First), (K::End, A::Last)],
    ),
    // PipeWire graph tab
    b(Scope::PipeWire, "j/k", "move", Some("move"), MOVE_ROWS),
    b(
        Scope::PipeWire,
        "d",
        "disconnect link",
        Some("disconnect"),
        &[(K::Char('d'), A::Disconnect), (K::Delete, A::Disconnect)],
    ),
    b(
        Scope::PipeWire,
        "a",
        "all links / Scarlett only",
        Some("all"),
        &[(K::Char('a'), A::ShowAll)],
    ),
    // Route picker (modal)
    b(Scope::Picker, "j/k", "move", Some("move"), MOVE_ROWS),
    b(
        Scope::Picker,
        "Enter",
        "set source",
        Some("set"),
        &[(K::Enter, A::Confirm), (K::Char(' '), A::Confirm)],
    ),
    b(
        Scope::Picker,
        "Esc",
        "cancel",
        Some("cancel"),
        &[(K::Esc, A::Cancel), (K::Char('q'), A::Cancel)],
    ),
    // Help overlay (modal)
    b(
        Scope::Help,
        "? Esc",
        "close help",
        Some("close help"),
        &[
            (K::Char('?'), A::Cancel),
            (K::Esc, A::Cancel),
            (K::Char('q'), A::Cancel),
        ],
    ),
    // Safety: documented here, handled by the armed-action logic.
    b(Scope::Safety, "Space Space", "enable 48V (I/O)", None, &[]),
    b(
        Scope::Safety,
        "0 0",
        "unity when the rise is > +6 dB",
        None,
        &[],
    ),
    b(Scope::Safety, "Esc", "cancel an armed action", None, &[]),
    // Everywhere
    b(
        Scope::Global,
        "1–5",
        "jump to tab",
        Some("tabs"),
        &[
            (K::Char('1'), A::GoTab(0)),
            (K::Char('2'), A::GoTab(1)),
            (K::Char('3'), A::GoTab(2)),
            (K::Char('4'), A::GoTab(3)),
            (K::Char('5'), A::GoTab(4)),
        ],
    ),
    b(
        Scope::Global,
        "Tab S-Tab",
        "next / previous tab",
        None,
        &[(K::Tab, A::NextTab), (K::BackTab, A::PrevTab)],
    ),
    b(
        Scope::Global,
        "r",
        "rescan device",
        Some("rescan"),
        &[(K::Char('r'), A::Rescan)],
    ),
    b(
        Scope::Global,
        "c",
        "switch card (2+ cards)",
        None,
        &[(K::Char('c'), A::SwitchCard)],
    ),
    b(
        Scope::Global,
        "?",
        "this help",
        None,
        &[(K::Char('?'), A::Help)],
    ),
    b(
        Scope::Global,
        "q",
        "quit (also Ctrl-C)",
        Some("quit"),
        &[(K::Char('q'), A::Quit)],
    ),
];

pub fn bindings(scope: Scope) -> impl Iterator<Item = &'static Binding> {
    KEYMAP.iter().filter(move |b| b.scope == scope)
}

fn find(scope: Scope, code: K) -> Option<Action> {
    bindings(scope)
        .flat_map(|b| b.keys.iter())
        .find(|(k, _)| *k == code)
        .map(|&(_, a)| a)
}

/// The action for `code` in `scope`, falling back to global keys unless the
/// scope is modal.
pub fn lookup(scope: Scope, code: K) -> Option<Action> {
    find(scope, code).or_else(|| match scope {
        Scope::Picker | Scope::Help => None,
        _ => find(Scope::Global, code),
    })
}

/// Footer hints for `scope` in priority order: the scope's own, then global.
/// The caller appends `? help` last.
pub fn hints(scope: Scope) -> Vec<(&'static str, &'static str)> {
    let own = bindings(scope).filter_map(|b| b.hint.map(|h| (b.label, h)));
    let global: Vec<_> = match scope {
        Scope::Picker | Scope::Help => Vec::new(),
        _ => bindings(Scope::Global)
            .filter_map(|b| b.hint.map(|h| (b.label, h)))
            .collect(),
    };
    own.chain(global).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const SCOPES: [Scope; 9] = [
        Scope::Global,
        Scope::Safety,
        Scope::Overview,
        Scope::Io,
        Scope::Routing,
        Scope::Matrix,
        Scope::PipeWire,
        Scope::Picker,
        Scope::Help,
    ];

    #[test]
    fn no_key_is_bound_twice_in_one_scope() {
        for scope in SCOPES {
            let mut seen: HashMap<K, &str> = HashMap::new();
            for b in bindings(scope) {
                for (k, _) in b.keys {
                    if let Some(prev) = seen.insert(*k, b.label) {
                        panic!("{scope:?}: {k:?} bound by both {prev:?} and {:?}", b.label);
                    }
                }
            }
        }
    }

    #[test]
    fn every_live_binding_has_keys_and_help() {
        for b in KEYMAP {
            assert!(!b.help.is_empty() && !b.label.is_empty());
            if b.scope != Scope::Safety {
                assert!(!b.keys.is_empty(), "{:?} {} has no keys", b.scope, b.label);
            }
        }
    }

    #[test]
    fn lookup_falls_back_to_global_except_in_modals() {
        assert_eq!(lookup(Scope::Io, K::Char('q')), Some(Action::Quit));
        assert_eq!(lookup(Scope::Io, K::Char('L')), Some(Action::IncBig));
        assert_eq!(lookup(Scope::Matrix, K::Char('3')), Some(Action::GoTab(2)));
        assert_eq!(lookup(Scope::Overview, K::Char('?')), Some(Action::Help));
        // Modal scopes: q cancels, digits do nothing.
        assert_eq!(lookup(Scope::Picker, K::Char('q')), Some(Action::Cancel));
        assert_eq!(lookup(Scope::Picker, K::Char('1')), None);
        assert_eq!(lookup(Scope::Help, K::Char('r')), None);
    }

    #[test]
    fn hints_lead_with_the_scope_then_global() {
        let h = hints(Scope::Matrix);
        assert_eq!(h.first(), Some(&("h j k l", "move")));
        assert!(h.contains(&("q", "quit")));
        let picker = hints(Scope::Picker);
        assert!(!picker.iter().any(|(_, w)| *w == "quit"));
    }
}
