use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match apk_lab_migrate::run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("apk-lab-migrate: {message}");
            ExitCode::from(2)
        }
    }
}
