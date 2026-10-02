use game_media_vault_application::{
    PortError, RecordedDerivative, VaultCatalogPort, VaultRepairCatalogPort,
};

use super::{SqliteCatalog, sql_error};

impl VaultCatalogPort for SqliteCatalog {
    fn referenced_originals(&self) -> Result<Vec<String>, PortError> {
        let connection = self.connect()?;
        let mut statement = connection
            .prepare(
                "SELECT DISTINCT object_hash
                 FROM assets
                 WHERE EXISTS (SELECT 1 FROM asset_provenance WHERE asset_id = assets.id)
                 ORDER BY object_hash",
            )
            .map_err(sql_error)?;
        statement
            .query_map([], |row| row.get(0))
            .map_err(sql_error)?
            .collect::<rusqlite::Result<_>>()
            .map_err(sql_error)
    }

    fn recorded_derivatives(&self) -> Result<Vec<RecordedDerivative>, PortError> {
        let connection = self.connect()?;
        let mut statement = connection
            .prepare(
                "SELECT original_hash, object_hash
                 FROM derived_objects
                 ORDER BY original_hash, recipe_key",
            )
            .map_err(sql_error)?;
        statement
            .query_map([], |row| {
                Ok(RecordedDerivative {
                    original_hash: row.get(0)?,
                    object_hash: row.get(1)?,
                })
            })
            .map_err(sql_error)?
            .collect::<rusqlite::Result<_>>()
            .map_err(sql_error)
    }
}

impl VaultRepairCatalogPort for SqliteCatalog {
    fn forget_derivatives(&self, object_hashes: &[String]) -> Result<(), PortError> {
        let object_hashes_json = serde_json::to_string(object_hashes)
            .map_err(|error| PortError::new(format!("failed to serialize hashes: {error}")))?;
        self.connect()?
            .execute(
                "DELETE FROM derived_objects
                 WHERE object_hash IN (SELECT value FROM json_each(?1))",
                [object_hashes_json],
            )
            .map_err(sql_error)?;
        Ok(())
    }
}
