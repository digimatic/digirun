use std::process::ExitCode;

use garmin_analyze::args;

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    match args::parse(&argv) {
        Ok(parsed) => ExitCode::from(garmin_analyze::run(&parsed) as u8),
        Err(error) => {
            if error.stdout {
                println!("{}", error.message);
            } else {
                eprintln!("{}", error.message);
            }
            ExitCode::from(error.code as u8)
        }
    }
}
