use std::path::Path;

use game_media_vault_application::list_library as list_library_use_case;
use game_media_vault_domain::LibraryEntry;
use game_media_vault_infrastructure::SqliteCatalog;

pub fn load_library(vault_root: &Path) -> Result<Vec<LibraryEntry>, String> {
    let catalog = SqliteCatalog::open_existing(vault_root.join("catalog.sqlite3"))
        .map_err(|error| error.to_string())?;
    list_library_use_case(&catalog).map_err(|error| error.to_string())
}

#[tauri::command(rename_all = "snake_case")]
fn list_library(vault_root: String) -> Result<Vec<LibraryEntry>, String> {
    load_library(Path::new(&vault_root))
}

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![list_library])
        .run(tauri::generate_context!())
        .expect("failed to run Game Media Vault");
}
