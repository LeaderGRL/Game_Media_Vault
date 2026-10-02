use std::{
    cell::RefCell,
    io::{Cursor, Read},
};

use game_media_vault_application::{
    CatalogPort, DerivativeRepositoryPort, DerivedStorePort, IncompletePackaging, OriginalObject,
    PackagingModelFailure, PackagingModelPort, PackagingScans, PortError, derive_packaging_models,
    list_library,
};
use game_media_vault_domain::{
    AssetProvenance, AssetType, AssetTypeSelector, DerivationRecipe, DerivedAsset, ImportedAsset,
    LibraryAsset, LibraryEntry, MediaInfo, PackagingTemplate, PersistAsset, SourceId, StoredObject,
};

const NES: &str = "Nintendo - Nintendo Entertainment System";

/// A library of releases whose originals are their hashes' bytes, with the derived outputs
/// stored and the derivatives recorded.
#[derive(Default)]
struct FakeVault {
    library: RefCell<Vec<LibraryEntry>>,
    stored: RefCell<Vec<Vec<u8>>>,
    recorded: RefCell<Vec<(String, DerivationRecipe, String)>>,
}

impl FakeVault {
    fn with_releases(releases: Vec<LibraryEntry>) -> Self {
        Self {
            library: RefCell::new(releases),
            ..Self::default()
        }
    }
}

impl CatalogPort for FakeVault {
    fn persist_asset(&self, _record: PersistAsset) -> Result<ImportedAsset, PortError> {
        unimplemented!("packaging models never persist Assets")
    }

    fn list_library(&self) -> Result<Vec<LibraryEntry>, PortError> {
        // The library lists recorded derivatives with the Assets of their original.
        let recorded = self.recorded.borrow();
        let mut library = self.library.borrow().clone();
        for asset in library.iter_mut().flat_map(|release| &mut release.assets) {
            asset.derived = recorded
                .iter()
                .filter(|(original, _, _)| *original == asset.object_hash)
                .map(|(_, recipe, output)| DerivedAsset {
                    recipe: recipe.clone(),
                    object_hash: output.clone(),
                    byte_len: 1,
                    media: MediaInfo::unknown(),
                })
                .collect();
        }
        Ok(library)
    }
}

impl DerivativeRepositoryPort for FakeVault {
    fn originals_without(
        &self,
        _recipe: &DerivationRecipe,
    ) -> Result<Vec<OriginalObject>, PortError> {
        unimplemented!("packaging models start from releases")
    }

    fn record_derivative(
        &self,
        original_hash: &str,
        recipe: &DerivationRecipe,
        output: &StoredObject,
    ) -> Result<(), PortError> {
        self.recorded.borrow_mut().push((
            original_hash.to_owned(),
            recipe.clone(),
            output.hash.clone(),
        ));
        Ok(())
    }
}

impl DerivedStorePort for FakeVault {
    fn open_original(&self, hash: &str) -> Result<Box<dyn Read + Send>, PortError> {
        Ok(Box::new(Cursor::new(hash.as_bytes().to_vec())))
    }

    fn store_derived(&self, reader: &mut dyn Read) -> Result<StoredObject, PortError> {
        let mut bytes = Vec::new();
        reader
            .read_to_end(&mut bytes)
            .map_err(|error| PortError::new(error.to_string()))?;
        let hash = format!("model-{}", String::from_utf8_lossy(&bytes));
        self.stored.borrow_mut().push(bytes);
        Ok(StoredObject {
            hash,
            byte_len: 1,
            media: MediaInfo::unknown(),
        })
    }
}

/// Joins the bytes and media types of the scans; fails on a front scan saying "broken".
struct FakeBuilder;

impl PackagingModelPort for FakeBuilder {
    fn build(
        &self,
        template: PackagingTemplate,
        scans: PackagingScans<'_>,
    ) -> Result<Vec<u8>, PortError> {
        assert_eq!(template, PackagingTemplate::CardboardBox);
        let mut parts = Vec::new();
        for scan in [scans.front, scans.back, scans.spine] {
            let mut bytes = String::new();
            scan.bytes
                .read_to_string(&mut bytes)
                .map_err(|error| PortError::new(error.to_string()))?;
            if bytes == "broken" {
                return Err(PortError::new("cannot decode the front".to_owned()));
            }
            parts.push(format!("{bytes}:{}", scan.media_type));
        }
        Ok(parts.join("+").into_bytes())
    }
}

fn scan(asset_id: i64, asset_type: AssetType, hash: &str) -> LibraryAsset {
    LibraryAsset {
        asset_id,
        asset_type,
        object_hash: hash.to_owned(),
        byte_len: 100,
        media: MediaInfo {
            media_type: "image/png".to_owned(),
            width: Some(1000),
            height: Some(1400),
        },
        original_filename: format!("{hash}.png"),
        provenance: vec![AssetProvenance {
            source_id: SourceId::from("fixture"),
            source_asset_label: None,
            source_location: format!("fixture://{hash}"),
            match_decision: None,
        }],
        derived: Vec::new(),
    }
}

fn release(release_edition_id: i64, platform: &str, assets: Vec<LibraryAsset>) -> LibraryEntry {
    LibraryEntry {
        game_id: release_edition_id,
        game_title: format!("Game {release_edition_id}"),
        release_edition_id,
        platform: platform.to_owned(),
        region: "USA".to_owned(),
        edition_name: "Standard".to_owned(),
        assertions: Vec::new(),
        assets,
    }
}

fn boxed_release(release_edition_id: i64, front: &str, back: &str, spine: &str) -> LibraryEntry {
    let id = release_edition_id * 10;
    release(
        release_edition_id,
        NES,
        vec![
            scan(id, AssetType::BoxFront, front),
            scan(id + 1, AssetType::BoxBack, back),
            scan(id + 2, AssetType::Spine, spine),
        ],
    )
}

fn model_recipe(back: &str, spine: &str) -> DerivationRecipe {
    DerivationRecipe::PackagingModel {
        template: PackagingTemplate::CardboardBox,
        back_hash: back.to_owned(),
        spine_hash: spine.to_owned(),
    }
}

#[test]
fn a_box_with_its_front_back_and_spine_gets_a_model_built_from_those_scans() {
    let vault = FakeVault::with_releases(vec![boxed_release(1, "front", "back", "spine")]);

    let summary = derive_packaging_models(&vault, &vault, &vault, &FakeBuilder).unwrap();

    assert_eq!(summary.generated, 1);
    assert!(summary.failed.is_empty());
    // The model is a Derived Asset of the front scan whose recipe names the other two exactly.
    assert_eq!(
        *vault.recorded.borrow(),
        vec![(
            "front".to_owned(),
            model_recipe("back", "spine"),
            "model-front:image/png+back:image/png+spine:image/png".to_owned(),
        )]
    );
}

#[test]
fn a_model_already_built_from_the_same_scans_is_reused() {
    let vault = FakeVault::with_releases(vec![
        boxed_release(1, "front", "back", "spine"),
        // Another release scanned with the same bytes shares the model.
        boxed_release(2, "front", "back", "spine"),
    ]);

    let first = derive_packaging_models(&vault, &vault, &vault, &FakeBuilder).unwrap();
    let again = derive_packaging_models(&vault, &vault, &vault, &FakeBuilder).unwrap();

    assert_eq!((first.generated, first.up_to_date), (1, 1));
    assert_eq!((again.generated, again.up_to_date), (0, 2));
    assert_eq!(vault.stored.borrow().len(), 1);
}

#[test]
fn a_new_scan_of_a_slot_builds_a_new_model() {
    let vault = FakeVault::with_releases(vec![boxed_release(1, "front", "back", "spine")]);
    derive_packaging_models(&vault, &vault, &vault, &FakeBuilder).unwrap();

    *vault.library.borrow_mut() = vec![boxed_release(1, "front", "rescanned-back", "spine")];
    let summary = derive_packaging_models(&vault, &vault, &vault, &FakeBuilder).unwrap();

    assert_eq!(summary.generated, 1);
    assert_eq!(
        vault.recorded.borrow().last().unwrap().1,
        model_recipe("rescanned-back", "spine")
    );
}

#[test]
fn a_box_missing_a_scan_explains_what_its_model_requires() {
    let vault = FakeVault::with_releases(vec![release(
        1,
        NES,
        vec![
            scan(10, AssetType::BoxFront, "front"),
            scan(11, AssetType::BoxBack, "back"),
        ],
    )]);

    let summary = derive_packaging_models(&vault, &vault, &vault, &FakeBuilder).unwrap();

    assert_eq!(summary.generated, 0);
    assert_eq!(
        summary.incomplete,
        vec![IncompletePackaging {
            release_edition_id: 1,
            missing: vec![AssetTypeSelector::Spine],
        }]
    );
    assert!(vault.stored.borrow().is_empty());
}

#[test]
fn packaging_without_a_template_builds_nothing() {
    let scans = |id: i64| {
        vec![
            scan(id, AssetType::BoxFront, &format!("front-{id}")),
            scan(id + 1, AssetType::BoxBack, &format!("back-{id}")),
            scan(id + 2, AssetType::Spine, &format!("spine-{id}")),
        ]
    };
    // A jewel case has no template yet, and an unknown platform no packaging family.
    let vault = FakeVault::with_releases(vec![
        release(1, "Sony - PlayStation", scans(10)),
        release(2, "Unknown Platform", scans(20)),
    ]);

    let summary = derive_packaging_models(&vault, &vault, &vault, &FakeBuilder).unwrap();

    assert_eq!((summary.generated, summary.without_template), (0, 2));
    assert!(summary.incomplete.is_empty());
}

#[test]
fn a_model_that_fails_to_build_is_reported_and_the_others_still_build() {
    let vault = FakeVault::with_releases(vec![
        boxed_release(1, "broken", "back", "spine"),
        boxed_release(2, "front", "back", "spine"),
    ]);

    let summary = derive_packaging_models(&vault, &vault, &vault, &FakeBuilder).unwrap();

    assert_eq!(summary.generated, 1);
    assert_eq!(
        summary.failed,
        vec![PackagingModelFailure {
            release_edition_id: 1,
            reason: "cannot decode the front".to_owned(),
        }]
    );
    assert_eq!(vault.stored.borrow().len(), 1);
}

#[test]
fn the_library_shows_the_model_of_the_current_preferred_scans() {
    let vault = FakeVault::with_releases(vec![boxed_release(1, "front", "back", "spine")]);
    let model_of = |vault: &FakeVault| {
        list_library(vault).unwrap()[0]
            .packaging_model
            .as_ref()
            .map(|model| model.object_hash.clone())
    };
    assert_eq!(model_of(&vault), None);

    derive_packaging_models(&vault, &vault, &vault, &FakeBuilder).unwrap();
    assert_eq!(
        model_of(&vault).as_deref(),
        Some("model-front:image/png+back:image/png+spine:image/png")
    );

    // A model of earlier scans is no longer the release's.
    *vault.library.borrow_mut() = vec![boxed_release(1, "front", "rescanned-back", "spine")];
    assert_eq!(model_of(&vault), None);
}
