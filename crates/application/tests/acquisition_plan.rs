use std::{
    io::Read,
    sync::atomic::{AtomicUsize, Ordering},
};

use game_media_vault_application::{
    AcquisitionPlan, ApplicationError, ConnectorPort, ErrorKind, ExcludedSource, PlannedSource,
    PortError, SelectorCoverage, plan_acquisition,
};
use game_media_vault_domain::{
    AcquisitionLimits, AcquisitionRequest, AcquisitionRequestDraft, AssetCandidate, AssetType,
    AssetTypeSelector, ConnectorCapabilities, GameSelection, QualityRequirements, RetentionPolicy,
    SourceSelection,
};

/// A connector that counts its plan checks; it is never asked to discover.
struct StubConnector {
    source_id: &'static str,
    asset_types: Vec<AssetType>,
    direct_media_download: bool,
    refusal: Option<&'static str>,
    consultations: AtomicUsize,
}

fn connector(source_id: &'static str, asset_types: Vec<AssetType>) -> StubConnector {
    StubConnector {
        source_id,
        asset_types,
        direct_media_download: true,
        refusal: None,
        consultations: AtomicUsize::new(0),
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
        self.consultations.fetch_add(1, Ordering::SeqCst);
        Ok(self.refusal.map(str::to_owned))
    }

    fn discover(&self, _request: &AcquisitionRequest) -> Result<Vec<AssetCandidate>, PortError> {
        unreachable!("planning never discovers")
    }

    fn download(&self, _candidate: &AssetCandidate) -> Result<Box<dyn Read + Send>, PortError> {
        unreachable!("planning never downloads")
    }
}

fn draft(sources: SourceSelection, asset_types: Vec<AssetTypeSelector>) -> AcquisitionRequestDraft {
    AcquisitionRequestDraft {
        sources,
        platforms: vec!["Nintendo - Game Boy".to_owned()],
        games: GameSelection::Explicit(vec!["Tetris (World) (Rev 1)".to_owned()]),
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types,
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    }
}

fn request(sources: SourceSelection, asset_types: Vec<AssetTypeSelector>) -> AcquisitionRequest {
    AcquisitionRequest::try_from_draft(draft(sources, asset_types)).unwrap()
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
    assert_eq!(other.consultations.load(Ordering::SeqCst), 0);
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
            vec![AssetTypeSelector::BoxFront, AssetTypeSelector::Screenshot],
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
    assert_eq!(unrelated.consultations.load(Ordering::SeqCst), 0);
}

#[test]
fn a_selected_source_without_a_connector_is_refused() {
    let libretro = connector("libretro-thumbnails", vec![AssetType::BoxFront]);
    let connectors: Vec<&dyn ConnectorPort> = vec![&libretro];

    let error = plan_acquisition(
        &request(
            SourceSelection::Explicit(vec!["screenscraper".to_owned()]),
            vec![AssetTypeSelector::BoxFront],
        ),
        &connectors,
    )
    .unwrap_err();

    assert!(
        matches!(error, ApplicationError::UnsupportedConnectorPlan { .. }),
        "{error}"
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
    assert_eq!(index.consultations.load(Ordering::SeqCst), 0);
}

fn uncovered(error: ApplicationError) -> (Vec<AssetTypeSelector>, Vec<ExcludedSource>) {
    match error {
        ApplicationError::UncoveredAssetTypes {
            uncovered,
            excluded,
        } => (uncovered, excluded),
        other => panic!("expected uncovered asset types, got {other}"),
    }
}

#[test]
fn a_requested_type_no_selected_source_acquires_is_refused_before_consulting_any() {
    let libretro = connector("libretro-thumbnails", vec![AssetType::BoxFront]);
    let connectors: Vec<&dyn ConnectorPort> = vec![&libretro];

    let error = plan_acquisition(
        &request(
            SourceSelection::Auto,
            vec![AssetTypeSelector::BoxFront, AssetTypeSelector::Manual],
        ),
        &connectors,
    )
    .unwrap_err();

    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert_eq!(
        uncovered(error),
        (vec![AssetTypeSelector::Manual], Vec::new())
    );
    assert_eq!(libretro.consultations.load(Ordering::SeqCst), 0);
}

#[test]
fn a_family_is_refused_since_it_still_selects_types_no_source_acquires() {
    let libretro = connector("libretro-thumbnails", vec![AssetType::BoxFront]);
    let connectors: Vec<&dyn ConnectorPort> = vec![&libretro];

    let error = plan_acquisition(
        &request(SourceSelection::Auto, vec![AssetTypeSelector::Packaging]),
        &connectors,
    )
    .unwrap_err();

    assert_eq!(uncovered(error).0, vec![AssetTypeSelector::Packaging]);
}

#[test]
fn a_type_left_uncovered_by_refusing_sources_is_refused_with_their_reasons() {
    let refusing = StubConnector {
        refusal: Some("no repository for Nintendo - Game Boy"),
        ..connector("libretro-thumbnails", vec![AssetType::BoxFront])
    };
    let unrelated = connector("snap-source", vec![AssetType::Screenshot]);
    let connectors: Vec<&dyn ConnectorPort> = vec![&refusing, &unrelated];

    let error = plan_acquisition(
        &request(SourceSelection::Auto, vec![AssetTypeSelector::BoxFront]),
        &connectors,
    )
    .unwrap_err();

    assert_eq!(
        error.to_string(),
        "no selected source acquires BoxFront (snap-source: acquires none of the requested \
         asset types; libretro-thumbnails: no repository for Nintendo - Game Boy)"
    );
}

#[test]
fn requirements_the_engine_cannot_apply_are_refused_before_consulting_any_source() {
    let libretro = connector("libretro-thumbnails", vec![AssetType::BoxFront]);
    let connectors: Vec<&dyn ConnectorPort> = vec![&libretro];
    let request = AcquisitionRequest::try_from_draft(AcquisitionRequestDraft {
        quality: Some(QualityRequirements {
            min_bitrate_kbps: Some(320),
            ..QualityRequirements::default()
        }),
        ..draft(SourceSelection::Auto, vec![AssetTypeSelector::BoxFront])
    })
    .unwrap();

    let error = plan_acquisition(&request, &connectors).unwrap_err();

    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert_eq!(libretro.consultations.load(Ordering::SeqCst), 0);
}

#[test]
fn a_source_selected_twice_is_planned_and_consulted_once() {
    let libretro = connector("libretro-thumbnails", vec![AssetType::BoxFront]);

    let plan = plan(
        &request(
            SourceSelection::Explicit(vec![
                "libretro-thumbnails".to_owned(),
                "libretro-thumbnails".to_owned(),
            ]),
            vec![AssetTypeSelector::BoxFront],
        ),
        &[&libretro],
    );

    assert_eq!(plan.sources.len(), 1);
    assert_eq!(libretro.consultations.load(Ordering::SeqCst), 1);
}
