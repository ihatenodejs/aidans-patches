use std::io::Write;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();

    match apk_lab::cli::run(&args) {
        Ok(code) => ExitCode::from(code.as_u8()),
        Err(error) if error.is_broken_pipe() => ExitCode::SUCCESS,
        Err(error) => {
            let mut stderr = std::io::stderr().lock();
            let _ = writeln!(stderr, "apk-lab: {error}");
            let _ = stderr.flush();
            ExitCode::from(error.exit_code().as_u8())
        }
    }
}
