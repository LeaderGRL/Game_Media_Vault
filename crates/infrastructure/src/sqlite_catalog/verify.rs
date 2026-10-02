use game_media_vault_application::{
    PortError, RecordedDerivative, UnfinishedWork, VaultCatalogPort, VaultRepairCatalogPort,
};
use game_media_vault_domain::DerivationRecipe;

use super::{SqliteCatalog, reviews::parse_review_status, runs::parse_run_status, sql_error};

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
                "SELECT original_hash, object_hash, recipe_json
                 FROM derived_objects
                 ORDER BY original_hash, recipe_key",
            )
            .map_err(sql_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(sql_error)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(sql_error)?;
        rows.into_iter()
            .map(|(original_hash, object_hash, recipe_json)| {
                let recipe: DerivationRecipe =
                    serde_json::from_str(&recipe_json).map_err(|error| {
                        PortError::new(format!("catalog contains an invalid recipe: {error}"))
                    })?;
                Ok(RecordedDerivative {
                    original_hash,
                    object_hash,
                    other_originals: recipe.other_originals(),
                })
            })
            .collect()
    }

    fn unfinished_work(&self) -> Result<Vec<UnfinishedWork>, PortError> {
        let connection = self.connect()?;
        let mut statement = connection
            .prepare(
                "SELECT work.run_id, run.status, work.work_key, review.status
                 FROM acquisition_run_work AS work
                 JOIN acquisition_runs AS run ON run.id = work.run_id
                 LEFT JOIN review_items AS review ON review.id = work.review_item_id
                 WHERE work.state IN ('queued', 'parked')
                 ORDER BY work.run_id, work.work_key",
            )
            .map_err(sql_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })
            .map_err(sql_error)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(sql_error)?;
        rows.into_iter()
            .map(|(run_id, run_status, work_key, review_status)| {
                Ok(UnfinishedWork {
                    run_id,
                    run_status: parse_run_status(&run_status)?,
                    work_key,
                    parked_on: review_status
                        .as_deref()
                        .map(parse_review_status)
                        .transpose()?,
                })
            })
            .collect()
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
