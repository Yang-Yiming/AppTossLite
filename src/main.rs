use clap::Parser;
use console::Style;

use toss::cli::{Cli, dispatch};

fn main() {
    let cli = Cli::parse();

    if let Err(e) = dispatch(cli) {
        let red = Style::new().red().bold();
        eprintln!("{} {}", red.apply_to("error:"), e);
        std::process::exit(1);
    }
}
