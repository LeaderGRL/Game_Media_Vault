fn main() {
    match game_media_vault_cli::run(std::env::args_os()) {
        Ok(output) if !output.is_empty() => println!("{output}"),
        Ok(_) => {}
        Err(game_media_vault_cli::CliError::Parse(error)) => error.exit(),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
