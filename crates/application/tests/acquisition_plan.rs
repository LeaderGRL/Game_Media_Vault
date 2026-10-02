use std::{cell::Cell, io::Read};

use game_media_vault_application::{
    AcquisitionPlan, ApplicationError, ConnectorPort, ExcludedSource, PlannedSource, PortError,
    SelectorCoverage, plan_acquisition,
};
use game_media_vault_domain::{
    AcquisitionLimits, AcquisitionRequest, AcquisitionRequestDraft, AssetCandidate, AssetType,
    AssetTypeSelector, ConnectorCapabilities, GameSelection, RetentionPolicy, SourceSelection,
};

/// A connector that records whether its plan check ran; it is never asked to discover.
struct StubConnector {
    source_id: &'static str,
    asset_types: Vec<AssetType>,
    direct_media_download: bool,
    refusal: Option<&'static str>,
    consulted: Cell<bool>,
}

fn connector(source_id: &'static str, asset_types: Vec<AssetType>) -> StubConnector {
    StubConnector {
        source_id,
        asset_types,
        direct_media_download: true,
        refusal: None,
        consulted: Cell::new(false),
    }
}

impl ConnectorPort for StubConnector {
    fn source_id(&self) -> &'static str {
        self.source_id
    }

    fn capabilities(&self) -> ConnectorCapabilities {
        ConnectorCapabilities {
            asset_types: self.asset_types.clone(),
            direct_media_download: self.direct_media_download,
        }
    }

    fn unsupported_request_reason(
        &self,
        _request: &AcquisitionRequest,
    ) -> Result<Option<String>, PortError> {
        self.consulted.set(true);
        Ok(self.refusal.map(str::to_owned))
    }

    fn discover(&self, _request: &AcquisitionRequest) -> Result<Vec<AssetCandidate>, PortError> {
        unreachable!("planning never discovers")
    }

    fn download(&self, _candidate: &AssetCandidate) -> Result<Box<dyn Read + Send>, PortError> {
        unreachable!("planning never downloads")
    }
}

fn request(sources: SourceSelection, asset_types: Vec<AssetTypeSelector>) -> AcquisitionRequest {
    AcquisitionRequest::try_from_draft(AcquisitionRequestDraft {
        sources,
        platforms: vec!["Nintendo - Game Boy".to_owned()],
        games: GameSelection::Explicit(vec!["Tetris (World) (Rev 1)".to_owned()]),
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types,
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    })
    .unwrap()
}

fn plan(request: &AcquisitionRequest, connectors: &[&StubConnector]) -> AcquisitionPlan {
    let connectors: Vec<&dyn ConnectorPort> = connectors
        .iter()
        .map(|connector| *connector as &dyn ConnectorPort)
        .collect();
    plan_acquisition(request, &connectors).unwrap()
}

#[test]
fn an_explicit_selection_contacts_only_the_selected_sources() {
    let libretro = connector("libretro-thumbnails", vec![AssetType::BoxFront]);
    let other = connector("screenscraper", vec![AssetType::BoxFront]);

    let plan = plan(
        &request(
            SourceSelection::Explicit(vec!["libretro-thumbnails".to_owned()]),
            vec![AssetTypeSelector::BoxFront],
        ),
        &[&libretro, &other],
    );

    assert_eq!(
        plan.sources,
        vec![PlannedSource {
            source_id: "libretro-thumbnails".to_owned(),
            asset_types: vec![AssetType::BoxFront],
        }]
    );
    assert!(!other.consulted.get());
}

#[test]
fn auto_fills_each_requested_type_from_the_sources_that_acquire_it() {
    let boxes = connector("box-source", vec![AssetType::BoxFront]);
    let snaps = connector(
        "snap-source",
        vec![AssetType::Screenshot, AssetType::TitleScreen],
    );

    let plan = plan(
        &request(
            SourceSelection::Auto,
            vec![
                AssetTypeSelector::BoxFront,
                AssetTypeSelector::Screenshot,
                AssetTypeSelector::Manual,
            ],
        ),
        &[&boxes, &snaps],
    );

    assert_eq!(
        plan.sources,
        vec![
            PlannedSource {
                source_id: "box-source".to_owned(),
                asset_types: vec![AssetType::BoxFront],
            },
            PlannedSource {
                source_id: "snap-source".to_owned(),
                asset_types: vec![AssetType::Screenshot],
            },
        ]
    );
    assert_eq!(
        plan.coverage,
        vec![
            SelectorCoverage {
                selector: AssetTypeSelector::BoxFront,
                sources: vec!["box-source".to_owned()],
            },
            SelectorCoverage {
                selector: AssetTypeSelector::Screenshot,
                sources: vec!["snap-source".to_owned()],
            },
            SelectorCoverage {
                selector: AssetTypeSelector::Manual,
                sources: Vec::new(),
            },
        ]
    );
}

#[test]
fn auto_leaves_out_sources_that_cannot_serve_the_request_and_says_why() {
    let unrelated = connector("snap-source", vec![AssetType::Screenshot]);
    let refusing = StubConnector {
        refusal: Some("this source does not cover Nintendo - Game Boy"),
        ..connector("box-source", vec![AssetType::BoxFront])
    };
    let serving = connector("libretro-thumbnails", vec![AssetType::BoxFront]);

    let plan = plan(
        &request(SourceSelection::Auto, vec![AssetTypeSelector::BoxFront]),
        &[&unrelated, &refusing, &serving],
    );

    assert_eq!(
        plan.sources
            .iter()
            .map(|source| source.source_id.as_str())
            .collect::<Vec<_>>(),
        ["libretro-thumbnails"]
    );
    assert_eq!(
        plan.excluded,
        vec![
            ExcludedSource {
                source_id: "snap-source".to_owned(),
                reason: "acquires none of the requested asset types".to_owned(),
            },
            ExcludedSource {
                source_id: "box-source".to_owned(),
                reason: "this source does not cover Nintendo - Game Boy".to_owned(),
            },
        ]
    );
    // A source that acquires none of the requested types is not consulted.
    assert!(!unrelated.consulted.get());
}

#[test]
fn a_selected_source_without_a_connector_or_a_plan_no_source_serves_is_refused() {
    let libretro = connector("libretro-thumbnails", vec![AssetType::BoxFront]);
    let connectors: Vec<&dyn ConnectorPort> = vec![&libretro];

    let unknown = plan_acquisition(
        &request(
            SourceSelection::Explicit(vec!["screenscraper".to_owned()]),
            vec![AssetTypeSelector::BoxFront],
        ),
        &connectors,
    )
    .unwrap_err();
    let unserved = plan_acquisition(
        &request(SourceSelection::Auto, vec![AssetTypeSelector::Manual]),
        &connectors,
    )
    .unwrap_err();

    assert!(
        matches!(unknown, ApplicationError::UnsupportedConnectorPlan { .. }),
        "{unknown}"
    );
    assert!(
        matches!(unserved, ApplicationError::NoSourceServesPlan { .. }),
        "{unserved}"
    );
    assert_eq!(
        unserved.to_string(),
        "no selected source can serve this plan \
         (libretro-thumbnails: acquires none of the requested asset types)"
    );
}

#[test]
fn a_source_that_cannot_download_media_is_left_out_without_being_consulted() {
    let index = StubConnector {
        direct_media_download: false,
        ..connector("index-source", vec![AssetType::BoxFront])
    };
    let libretro = connector("libretro-thumbnails", vec![AssetType::BoxFront]);
    let connectors: Vec<&dyn ConnectorPort> = vec![&index, &libretro];

    let plan = plan_acquisition(
        &request(SourceSelection::Auto, vec![AssetTypeSelector::BoxFront]),
        &connectors,
    )
    .unwrap();

    assert_eq!(
        plan.excluded,
        vec![ExcludedSource {
            source_id: "index-source".to_owned(),
            reason: "cannot download media directly".to_owned(),
        }]
    );
    assert!(!index.consulted.get());
}
