mod support;

use std::io::{Cursor, Read};

use game_media_vault_application::{
    ConnectorPort, DownloadLimits, PortError, RunRepositoryPort, acquire_run_with_connectors,
    plan_acquisition,
};
use game_media_vault_domain::{
    AcquisitionRequest, AcquisitionRequestDraft, AssetCandidate, AssetType, ConnectorCapabilities,
    GameSelection, PlatformBoundGameSelector, SourceId, SourceSelection,
};
use support::*;

const NES: &str = "Nintendo - Nintendo Entertainment System";

/// A Source that cannot tell media of regions apart, as Libretro Thumbnails, whose files are
/// named after releases.
struct RegionBlindSource {
    /// The regions each discovery was asked to keep to.
    discovered_regions: Shared<Vec<Vec<String>>>,
}

impl RegionBlindSource {
    fn new() -> Self {
        Self {
            discovered_regions: Shared::new(Vec::new()),
        }
    }
}

impl ConnectorPort for RegionBlindSource {
    fn source_id(&self) -> &'static str {
        "region-blind"
    }

    fn capabilities(&self) -> ConnectorCapabilities {
        ConnectorCapabilities {
            asset_types: vec![AssetType::BoxFront],
            direct_media_download: true,
        }
    }

    fn unsupported_request_reason(
        &self,
        request: &AcquisitionRequest,
    ) -> Result<Option<String>, PortError> {
        Ok((!request.regions().is_empty()).then(|| "cannot tell regions apart".to_owned()))
    }

    fn discover(&self, request: &AcquisitionRequest) -> Result<Vec<AssetCandidate>, PortError> {
        self.discovered_regions
            .borrow_mut()
            .push(request.regions().to_vec());
        Ok(vec![AssetCandidate {
            source_id: SourceId::from("region-blind"),
            ..candidate("Tetris")
        }])
    }

    fn download(&self, candidate: &AssetCandidate) -> Result<Box<dyn Read + Send>, PortError> {
        Ok(Box::new(Cursor::new(
            candidate.source_url.clone().into_bytes(),
        )))
    }
}

/// Box Fronts of `games` on the NES, kept to Europe.
fn european(games: &[&str]) -> AcquisitionRequest {
    AcquisitionRequest::try_from_draft(AcquisitionRequestDraft {
        sources: SourceSelection::Auto,
        regions: vec!["Europe".to_owned()],
        games: GameSelection::PlatformBound(
            games
                .iter()
                .map(|game| PlatformBoundGameSelector {
                    game: (*game).to_owned(),
                    platform: NES.to_owned(),
                })
                .collect(),
        ),
        ..request_draft()
    })
    .unwrap()
}

#[test]
fn a_source_blind_to_regions_serves_games_whose_names_carry_the_requested_region() {
    let source = RegionBlindSource::new();
    let connectors: Vec<&dyn ConnectorPort> = vec![&source];

    // Each name keeps to Europe already: a worldwide release is released there too.
    let plan = plan_acquisition(
        &european(&["Tetris (Europe)", "Zelda (USA, Europe)", "Mario (World)"]),
        &connectors,
    )
    .unwrap();

    assert_eq!(plan.sources.len(), 1);
}

#[test]
fn a_source_blind_to_regions_is_left_out_when_a_name_does_not_say_its_region() {
    let source = RegionBlindSource::new();
    let connectors: Vec<&dyn ConnectorPort> = vec![&source];

    let error =
        plan_acquisition(&european(&["Tetris (Europe)", "Tetris"]), &connectors).unwrap_err();

    assert!(
        error.to_string().contains("cannot tell regions apart"),
        "{error}"
    );
}

#[test]
fn a_source_blind_to_regions_discovers_the_games_named_without_the_region_filter() {
    let source = RegionBlindSource::new();
    let vault = FakeVault::default();
    let run_id = vault
        .create_run(
            european(&["Tetris (Europe)"]),
            vec!["region-blind".to_owned()],
        )
        .unwrap()
        .id;

    acquire_run_with_connectors(
        &vault,
        &vault,
        &vault,
        &FakeStore::default(),
        &[&source as &dyn ConnectorPort],
        run_id,
        matching_policy(),
        DownloadLimits::default(),
    )
    .unwrap();

    assert_eq!(*source.discovered_regions.borrow(), [Vec::<String>::new()]);
}
