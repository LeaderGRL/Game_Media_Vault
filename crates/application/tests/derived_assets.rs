use std::{
    cell::RefCell,
    collections::BTreeMap,
    io::{Cursor, Read},
};

use game_media_vault_application::{
    DerivationFailure, DerivativeRepositoryPort, DerivedStorePort, MediaTransformPort,
    OriginalObject, PortError, derive_assets,
};
use game_media_vault_domain::{DerivationRecipe, MediaInfo, StoredObject};

const THUMBNAIL: DerivationRecipe = DerivationRecipe::Thumbnail { max_edge: 256 };

/// Originals by hash, derived outputs stored and the derivatives recorded for them.
#[derive(Default)]
struct FakeVault {
    originals: BTreeMap<String, (Vec<String>, Vec<u8>)>,
    stored: RefCell<Vec<Vec<u8>>>,
    recorded: RefCell<Vec<(String, DerivationRecipe, String)>>,
}

impl FakeVault {
    fn with_originals(originals: &[(&str, &[&str], &[u8])]) -> Self {
        Self {
            originals: originals
                .iter()
                .map(|(hash, media_types, bytes)| {
                    let media_types = media_types.iter().map(|&media_type| media_type.to_owned());
                    ((*hash).to_owned(), (media_types.collect(), bytes.to_vec()))
                })
                .collect(),
            ..Self::default()
        }
    }
}

impl DerivativeRepositoryPort for FakeVault {
    fn originals_without(
        &self,
        recipe: &DerivationRecipe,
    ) -> Result<Vec<OriginalObject>, PortError> {
        let recorded = self.recorded.borrow();
        Ok(self
            .originals
            .iter()
            .filter(|(hash, _)| {
                !recorded
                    .iter()
                    .any(|(original, done, _)| original == *hash && done == recipe)
            })
            .map(|(hash, (media_types, _))| OriginalObject {
                hash: hash.clone(),
                media_types: media_types.clone(),
            })
            .collect())
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
        let (_, bytes) = self
            .originals
            .get(hash)
            .ok_or_else(|| PortError::new(format!("object {hash} is missing")))?;
        Ok(Box::new(Cursor::new(bytes.clone())))
    }

    fn store_derived(&self, reader: &mut dyn Read) -> Result<StoredObject, PortError> {
        let mut bytes = Vec::new();
        reader
            .read_to_end(&mut bytes)
            .map_err(|error| PortError::new(error.to_string()))?;
        let hash = format!("derived-{}", String::from_utf8_lossy(&bytes));
        self.stored.borrow_mut().push(bytes);
        Ok(StoredObject {
            hash,
            byte_len: 3,
            media: MediaInfo::unknown(),
        })
    }
}

/// Uppercases PNG and JPEG bytes; fails on bytes saying "broken" and on media types it cannot read.
struct FakeTransformer;

impl MediaTransformPort for FakeTransformer {
    fn can_transform(&self, media_type: &str, _recipe: &DerivationRecipe) -> bool {
        matches!(media_type, "image/png" | "image/jpeg")
    }

    fn transform(
        &self,
        original: &mut dyn Read,
        media_type: &str,
        recipe: &DerivationRecipe,
    ) -> Result<Vec<u8>, PortError> {
        if !self.can_transform(media_type, recipe) {
            return Err(PortError::new(format!("cannot decode {media_type}")));
        }
        let mut bytes = Vec::new();
        original
            .read_to_end(&mut bytes)
            .map_err(|error| PortError::new(error.to_string()))?;
        if bytes == b"broken" {
            return Err(PortError::new("cannot decode the image".to_owned()));
        }
        Ok(bytes.to_ascii_uppercase())
    }
}

#[test]
fn derives_each_original_once_and_reuses_recorded_outputs() {
    let vault = FakeVault::with_originals(&[
        ("aaa", &["image/png"], b"one"),
        ("bbb", &["image/jpeg"], b"two"),
    ]);

    let summary = derive_assets(&vault, &vault, &FakeTransformer, &THUMBNAIL).unwrap();

    assert_eq!(summary.derived, 2);
    assert!(summary.failed.is_empty());
    assert_eq!(
        *vault.recorded.borrow(),
        vec![
            ("aaa".to_owned(), THUMBNAIL, "derived-ONE".to_owned()),
            ("bbb".to_owned(), THUMBNAIL, "derived-TWO".to_owned()),
        ]
    );
    let again = derive_assets(&vault, &vault, &FakeTransformer, &THUMBNAIL).unwrap();
    assert_eq!(again.derived, 0);
}

#[test]
fn a_failed_transform_is_reported_and_the_others_still_derive() {
    let vault = FakeVault::with_originals(&[
        ("aaa", &["image/png"], b"broken"),
        ("bbb", &["image/png"], b"two"),
    ]);

    let summary = derive_assets(&vault, &vault, &FakeTransformer, &THUMBNAIL).unwrap();

    assert_eq!(summary.derived, 1);
    assert_eq!(
        summary.failed,
        vec![DerivationFailure {
            original_hash: "aaa".to_owned(),
            reason: "cannot decode the image".to_owned(),
        }]
    );
    // Only the successful output was stored.
    assert_eq!(*vault.stored.borrow(), vec![b"TWO".to_vec()]);
}

#[test]
fn originals_the_transformer_cannot_read_are_skipped() {
    let vault = FakeVault::with_originals(&[
        ("aaa", &["application/pdf"], b"manual"),
        ("bbb", &["image/png"], b"two"),
    ]);

    let summary = derive_assets(&vault, &vault, &FakeTransformer, &THUMBNAIL).unwrap();

    assert_eq!((summary.derived, summary.skipped), (1, 1));
    assert!(summary.failed.is_empty());
}

#[test]
fn an_original_shared_by_assets_of_several_media_types_derives_from_one_it_can_read() {
    // Bytes shared with an Asset whose media type was never identified.
    let vault =
        FakeVault::with_originals(&[("aaa", &["application/octet-stream", "image/png"], b"one")]);

    let summary = derive_assets(&vault, &vault, &FakeTransformer, &THUMBNAIL).unwrap();

    assert_eq!((summary.derived, summary.skipped), (1, 0));
    assert!(summary.failed.is_empty(), "{:?}", summary.failed);
}
