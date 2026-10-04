mod common;
mod daemon;
mod tui;

const HELP: &str = "mouzi: file organizer for Linux\n\nUSAGE:\n  mouzi           open the TUI\n  mouzi daemon    run the background organizer (use the systemd user unit)\n  mouzi --version\n";

fn main() {
    let result = match std::env::args().nth(1).as_deref() {
        None => tui::run(),
        Some("daemon") => daemon::run(),
        Some("--version" | "-V") => {
            println!("mouzi {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some(_) => {
            print!("{HELP}");
            Ok(())
        }
    };
    if let Err(e) = result {
        eprintln!("mouzi: {e}");
        std::process::exit(1);
    }
}
