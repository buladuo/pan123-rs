mod cli;
mod icons;

use cli::Pan123Cli;

fn main() {
    if let Err(err) = Pan123Cli::run_from_env() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}
