use std::process::ExitCode;

fn main() -> ExitCode {
    // A simple way to check for a GUI flag without a full clap parse.
    if std::env::args().any(|arg| arg == "--gui") {
        if let Err(e) = motoc::gui::run_gui() {
            eprintln!("Error running GUI: {}", e);
            return ExitCode::FAILURE;
        }
        ExitCode::SUCCESS
    } else {
        motoc::cli::run()
    }
}
