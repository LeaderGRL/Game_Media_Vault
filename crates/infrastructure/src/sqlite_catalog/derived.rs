use std::collections::HashMap;

use game_media_vault_application::{DerivativeRepositoryPort, OriginalObject, PortError};
use game_media_vault_domain::{DerivationRecipe, DerivedAsset, MediaInfo, StoredObject};
use rusqlite::{Connection, params};

use super::{SqliteCatalog, sql_error};

impl DerivativeRepositoryPort for SqliteCatalog {
    fn originals_without(
        &self,
        recipe: &DerivationRecipe,
    ) -> Result<Vec<OriginalObject>, PortError> {
        let connection = self.connect()?;
        let mut statement = connection
            .prepare(
                "SELECT DISTINCT object_hash, media_type
                 FROM assets
                 WHERE EXISTS (SELECT 1 FROM asset_provenance WHERE asset_id = assets.id)
                   AND NOT EXISTS (
                       SELECT 1 FROM derived_objects
                       WHERE original_hash = assets.object_hash AND recipe_key = ?1
                   )
                 ORDER BY object_hash, media_type",
            )
            .map_err(sql_error)?;
        let mut rows = statement.query(params![recipe.key()]).map_err(sql_error)?;
        let mut originals: Vec<OriginalObject> = Vec::new();
        while let Some(row) = rows.next().map_err(sql_error)? {
            let hash: String = row.get(0).map_err(sql_error)?;
            let media_type: String = row.get(1).map_err(sql_error)?;
            match originals.last_mut() {
                Some(original) if original.hash == hash => original.media_types.push(media_type),
                _ => originals.push(OriginalObject {
                    hash,
                    media_types: vec![media_type],
                }),
            }
        }
        Ok(originals)
    }

    fn record_derivative(
        &self,
        original_hash: &str,
        recipe: &DerivationRecipe,
        output: &StoredObject,
    ) -> Result<(), PortError> {
        let recipe_json = serde_json::to_string(recipe)
            .map_err(|error| PortError::new(format!("failed to serialize a recipe: {error}")))?;
        let byte_len = i64::try_from(output.byte_len)
            .map_err(|_| PortError::new("derived output is too large".to_owned()))?;
        self.connect()?
            .execute(
                "INSERT OR IGNORE INTO derived_objects
                     (original_hash, recipe_key, recipe_json, object_hash, byte_len, media_type,
                      width, height)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    original_hash,
                    recipe.key(),
                    recipe_json,
                    output.hash,
                    byte_len,
                    output.media.media_type,
                    output.media.width,
                    output.media.height
                ],
            )
            .map_err(sql_error)?;
        Ok(())
    }
}

/// The Derived Assets of every original, in recipe order.
pub(super) fn derived_by_original(
    connection: &Connection,
    latest: Option<usize>,
) -> Result<HashMap<String, Vec<DerivedAsset>>, PortError> {
    let filter = match latest {
        None => "",
        Some(_) => {
            "WHERE original_hash IN (SELECT object_hash FROM assets ORDER BY id DESC LIMIT ?1)"
        }
    };
    let mut statement = connection
        .prepare(&format!(
            "SELECT original_hash, recipe_json, object_hash, byte_len, media_type, width, height
             FROM derived_objects
             {filter}
             ORDER BY original_hash, recipe_key"
        ))
        .map_err(sql_error)?;
    let mut rows = match latest {
        None => statement.query([]),
        Some(limit) => statement.query([i64::try_from(limit).unwrap_or(i64::MAX)]),
    }
    .map_err(sql_error)?;
    let mut derived: HashMap<String, Vec<DerivedAsset>> = HashMap::new();
    while let Some(row) = rows.next().map_err(sql_error)? {
        let recipe_json: String = row.get(1).map_err(sql_error)?;
        let recipe = serde_json::from_str(&recipe_json).map_err(|error| {
            PortError::new(format!("catalog contains an invalid recipe: {error}"))
        })?;
        let byte_len: i64 = row.get(3).map_err(sql_error)?;
        derived
            .entry(row.get(0).map_err(sql_error)?)
            .or_default()
            .push(DerivedAsset {
                recipe,
                object_hash: row.get(2).map_err(sql_error)?,
                byte_len: u64::try_from(byte_len).map_err(|_| {
                    PortError::new("catalog contains a negative byte length".into())
                })?,
                media: MediaInfo {
                    media_type: row.get(4).map_err(sql_error)?,
                    width: row.get(5).map_err(sql_error)?,
                    height: row.get(6).map_err(sql_error)?,
                    document: None,
                },
            });
    }
    Ok(derived)
}
