mod cli;
mod icons;

use cli::Pan123Cli;

fn main() {
    // Increase stack size to prevent overflow in upload operations
    std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024) // 8 MB stack
        .spawn(|| {
            if let Err(err) = Pan123Cli::run_from_env() {
                eprintln!("error: {err}");
                std::process::exit(1);
            }
        })
        .expect("Failed to spawn main thread")
        .join()
        .expect("Main thread panicked");
}
