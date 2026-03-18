use clap::Parser;
use console::Style;

use toss::cli::{Cli, dispatch};

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Some(cmd) => dispatch(cmd),
        None => toss::tui::run(),
    };

    if let Err(e) = result {
        let red = Style::new().red().bold();
        eprintln!("{} {}", red.apply_to("error:"), e);
        std::process::exit(1);
    }
}
