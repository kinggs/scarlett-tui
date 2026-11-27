mod alsa_ctl;
mod daemon;
mod keys;
mod model;
mod pipewire;
mod ui;

use std::env;
use std::process;

fn print_help() {
    println!(
        "scarlett-tui — Terminal mixer, router, live meters, and Omarchy plugin backend for Focusrite Scarlett\n\n\
         USAGE:\n  \
           scarlett-tui [--card N]           Launch interactive TUI\n  \
           scarlett-tui --json [--card N]    Print current hardware state as JSON and exit\n  \
           scarlett-tui --daemon [--card N]  Run JSON-lines daemon (used by Omarchy bar plugin)\n  \
           scarlett-tui --list               List detected Focusrite USB cards\n  \
           scarlett-tui --help               Show this help"
    );
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut mode = "tui";
    let mut card_override: Option<u32> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                print_help();
                return;
            }
            "--json" => mode = "json",
            "--daemon" => mode = "daemon",
            "--list" => mode = "list",
            "--card" => {
                if let Some(val) = args.get(i + 1)
                    && let Ok(n) = val.parse::<u32>()
                {
                    card_override = Some(n);
                    i += 1;
                } else {
                    eprintln!("--card requires a numeric ALSA card index");
                    process::exit(2);
                }
            }
            other => {
                eprintln!("Unknown argument: {other}");
                process::exit(2);
            }
        }
        i += 1;
    }

    match mode {
        "list" => {
            let cards = alsa_ctl::discover_focusrite_cards();
            if cards.is_empty() {
                println!("No Focusrite USB interfaces found.");
            } else {
                for c in cards {
                    println!(
                        "card {} [{}]: {} ({}, USB {})",
                        c.index, c.alsa_id, c.name, c.generation, c.usb_id
                    );
                }
            }
        }
        "json" => process::exit(daemon::run_once_json(card_override)),
        "daemon" => process::exit(daemon::run_daemon(card_override)),
        _ => {
            if let Err(e) = ui::run_tui(card_override) {
                eprintln!("scarlett-tui error: {e}");
                process::exit(1);
            }
        }
    }
}
