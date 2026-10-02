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
    fn forget_derivatives(&self, derivatives: &[RecordedDerivative]) -> Result<(), PortError> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction().map_err(sql_error)?;
        {
            let mut statement = transaction
                .prepare(
                    "DELETE FROM derived_objects
                     WHERE original_hash = ?1 AND object_hash = ?2",
                )
                .map_err(sql_error)?;
            for derivative in derivatives {
                statement
                    .execute([&derivative.original_hash, &derivative.object_hash])
                    .map_err(sql_error)?;
            }
        }
        transaction.commit().map_err(sql_error)
    }
}
