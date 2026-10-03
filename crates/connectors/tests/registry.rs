use std::sync::Arc;

use game_media_vault_connectors::{
    LAUNCHBOX_GAMES_DB_SOURCE_ID, LIBRETRO_THUMBNAILS_SOURCE_ID, PSX_DATACENTER_SOURCE_ID,
    STEAMGRIDDB_SOURCE_ID, THEGAMESDB_SOURCE_ID, registered_connectors,
};
use game_media_vault_infrastructure::NoCredentials;

#[test]
fn every_implemented_source_is_registered_once() {
    let source_ids: Vec<&str> = registered_connectors(Arc::new(NoCredentials))
        .iter()
        .map(|connector| connector.source_id())
        .collect();

    assert_eq!(
        source_ids,
        [
            LIBRETRO_THUMBNAILS_SOURCE_ID,
            LAUNCHBOX_GAMES_DB_SOURCE_ID,
            STEAMGRIDDB_SOURCE_ID,
            THEGAMESDB_SOURCE_ID,
            PSX_DATACENTER_SOURCE_ID,
        ]
    );
}
