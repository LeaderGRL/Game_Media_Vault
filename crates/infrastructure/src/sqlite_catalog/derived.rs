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
                "SELECT object_hash, MIN(media_type)
                 FROM assets
                 WHERE EXISTS (SELECT 1 FROM asset_provenance WHERE asset_id = assets.id)
                   AND NOT EXISTS (
                       SELECT 1 FROM derived_objects
                       WHERE original_hash = assets.object_hash AND recipe_key = ?1
                   )
                 GROUP BY object_hash
                 ORDER BY object_hash",
            )
            .map_err(sql_error)?;
        statement
            .query_map(params![recipe.key()], |row| {
                Ok(OriginalObject {
                    hash: row.get(0)?,
                    media_type: row.get(1)?,
                })
            })
            .map_err(sql_error)?
            .collect::<rusqlite::Result<_>>()
            .map_err(sql_error)
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
) -> Result<HashMap<String, Vec<DerivedAsset>>, PortError> {
    let mut statement = connection
        .prepare(
            "SELECT original_hash, recipe_json, object_hash, byte_len, media_type, width, height
             FROM derived_objects
             ORDER BY original_hash, recipe_key",
        )
        .map_err(sql_error)?;
    let mut rows = statement.query([]).map_err(sql_error)?;
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
                },
            });
    }
    Ok(derived)
}
