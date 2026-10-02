fn main() {
    match game_media_vault_cli::run(std::env::args_os()) {
        Ok(output) if !output.is_empty() => println!("{output}"),
        Ok(_) => {}
        Err(game_media_vault_cli::CliError::Parse(error)) => error.exit(),
        Err(error) => {
            // A failed execution still prints the run it left, so scripts can read it.
            if let Some(output) = error.output() {
                println!("{output}");
            }
            eprintln!("{error}");
            std::process::exit(error.exit_code());
        }
    }
}
