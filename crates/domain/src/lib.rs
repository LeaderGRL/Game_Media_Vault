use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", content = "values", rename_all = "snake_case")]
pub enum SourceSelection {
    Auto,
    Explicit(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", content = "values", rename_all = "snake_case")]
pub enum GameSelection {
    All,
    Explicit(Vec<String>),
    PlatformBound(Vec<PlatformBoundGameSelector>),
    QueryResult(Vec<PlatformBoundGameSelector>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformBoundGameSelector {
    pub game: String,
    pub platform: String,
}

impl GameSelection {
    fn is_valid(&self) -> bool {
        match self {
            Self::All => true,
            Self::Explicit(values) => {
                !values.is_empty() && values.iter().all(|value| !value.trim().is_empty())
            }
            Self::PlatformBound(values) | Self::QueryResult(values) => {
                !values.is_empty()
                    && values.iter().all(|value| {
                        !value.game.trim().is_empty() && !value.platform.trim().is_empty()
                    })
            }
        }
    }

    fn fixes_platforms_explicitly(&self) -> bool {
        matches!(
            self,
            Self::PlatformBound(values) | Self::QueryResult(values)
                if !values.is_empty()
                    && values.iter().all(|value| !value.platform.trim().is_empty())
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetTypeSelector {
    Packaging,
    PhysicalMedia,
    Documentation,
    DigitalMedia,
    PromotionalAndHistorical,
    HardwareArcade,
    OtherFamily,
    BoxFront,
    BoxBack,
    Spine,
    InnerCover,
    BoxTexture,
    #[serde(rename = "box_3d_render")]
    Box3dRender,
    #[serde(rename = "box_3d_model")]
    Box3dModel,
    SlipcoverSleeve,
    Insert,
    Cartridge,
    CartridgeFront,
    CartridgeBack,
    CartridgeLabel,
    Disc,
    DiscFront,
    DiscBack,
    DiscLabel,
    Pcb,
    CassetteTape,
    FloppyDisk,
    Manual,
    ManualPage,
    StrategyGuide,
    Map,
    ReferenceCard,
    RegistrationCard,
    WarrantySafetyInsert,
    Screenshot,
    TitleScreen,
    GameplayVideo,
    Trailer,
    Logo,
    Icon,
    WallpaperArtwork,
    Flyer,
    Advertisement,
    Poster,
    PromotionalArtwork,
    PressMaterial,
    MagazineScan,
    ArcadeCabinet,
    ControlPanel,
    Marquee,
    Bezel,
    Controller,
    Accessory,
    Soundtrack,
    Texture,
    #[serde(rename = "3d_model")]
    Model3d,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetentionPolicy {
    KeepEverything,
    KeepBestPerType,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct QualityRequirements {
    pub min_width: Option<u32>,
    pub min_height: Option<u32>,
    pub min_longest_edge: Option<u32>,
    pub min_pixel_count: Option<u64>,
    pub original_only: bool,
    pub accepted_mime_types: Vec<String>,
    pub max_compression_ratio: Option<u32>,
    pub min_bitrate_kbps: Option<u32>,
    pub preferred_scan_type: Option<String>,
    pub preferred_source_priority: Vec<String>,
    pub best_available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AcquisitionLimits {
    pub max_games: Option<u32>,
    pub max_downloads: Option<u32>,
    pub max_concurrent_downloads: Option<u16>,
    pub max_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AcquisitionRequest {
    sources: SourceSelection,
    platforms: Vec<String>,
    games: GameSelection,
    regions: Vec<String>,
    languages: Vec<String>,
    asset_types: Vec<AssetTypeSelector>,
    quality: Option<QualityRequirements>,
    retention: RetentionPolicy,
    limits: AcquisitionLimits,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcquisitionRequestDraft {
    pub sources: SourceSelection,
    pub platforms: Vec<String>,
    pub games: GameSelection,
    pub regions: Vec<String>,
    pub languages: Vec<String>,
    pub asset_types: Vec<AssetTypeSelector>,
    pub quality: Option<QualityRequirements>,
    pub retention: RetentionPolicy,
    pub limits: AcquisitionLimits,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcquisitionRunStatus {
    Running,
    Paused,
    Cancelled,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AcquisitionRun {
    pub id: i64,
    pub request: AcquisitionRequest,
    pub status: AcquisitionRunStatus,
    pub queued_work: u64,
    pub completed_work: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcquisitionWorkItem {
    pub key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AcquisitionRequestValidationError {
    MissingSources,
    MissingAssetTypes,
    MissingPlatforms,
    InvalidGameSelection,
}

impl fmt::Display for AcquisitionRequestValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingSources => {
                formatter.write_str("acquisition request must include at least one source")
            }
            Self::MissingAssetTypes => {
                formatter.write_str("acquisition request must include at least one asset type")
            }
            Self::MissingPlatforms => {
                formatter.write_str("acquisition request must include at least one platform")
            }
            Self::InvalidGameSelection => {
                formatter.write_str("acquisition request contains an invalid game selection")
            }
        }
    }
}

impl std::error::Error for AcquisitionRequestValidationError {}

fn non_blank_values(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .filter(|value| !value.trim().is_empty())
        .collect()
}

impl QualityRequirements {
    fn normalized(mut self) -> Self {
        self.accepted_mime_types = non_blank_values(self.accepted_mime_types);
        self.preferred_scan_type = self
            .preferred_scan_type
            .filter(|value| !value.trim().is_empty());
        self.preferred_source_priority = non_blank_values(self.preferred_source_priority);
        self
    }
}

impl AcquisitionRequest {
    pub fn platforms(&self) -> &[String] {
        &self.platforms
    }

    pub fn games(&self) -> &GameSelection {
        &self.games
    }

    pub fn regions(&self) -> &[String] {
        &self.regions
    }

    pub fn languages(&self) -> &[String] {
        &self.languages
    }

    pub fn quality(&self) -> Option<&QualityRequirements> {
        self.quality.as_ref()
    }

    pub fn retention(&self) -> RetentionPolicy {
        self.retention
    }

    pub fn limits(&self) -> &AcquisitionLimits {
        &self.limits
    }

    pub fn selects_only_source(&self, source_id: &str) -> bool {
        matches!(
            &self.sources,
            SourceSelection::Explicit(values)
                if values.len() == 1 && values[0] == source_id
        )
    }

    pub fn selects_source(&self, source_id: &str) -> bool {
        match &self.sources {
            SourceSelection::Auto => true,
            SourceSelection::Explicit(values) => values.iter().any(|value| value == source_id),
        }
    }

    pub fn requests_asset_type(&self, asset_type: AssetType) -> bool {
        self.asset_types.iter().any(|selector| {
            matches!(
                (selector, asset_type),
                (AssetTypeSelector::Packaging, AssetType::BoxFront)
                    | (AssetTypeSelector::BoxFront, AssetType::BoxFront)
            )
        })
    }

    pub fn requested_asset_types_supported_by(&self, supported: &[AssetType]) -> bool {
        self.asset_types.iter().all(|selector| match selector {
            AssetTypeSelector::Packaging | AssetTypeSelector::BoxFront => {
                supported.contains(&AssetType::BoxFront)
            }
            _ => false,
        })
    }

    pub fn try_from_draft(
        draft: AcquisitionRequestDraft,
    ) -> Result<Self, AcquisitionRequestValidationError> {
        let sources = match draft.sources {
            SourceSelection::Auto => SourceSelection::Auto,
            SourceSelection::Explicit(values) => {
                let values = non_blank_values(values);
                if values.is_empty() {
                    return Err(AcquisitionRequestValidationError::MissingSources);
                }
                SourceSelection::Explicit(values)
            }
        };
        if draft.asset_types.is_empty() {
            return Err(AcquisitionRequestValidationError::MissingAssetTypes);
        }
        if !draft.games.is_valid() {
            return Err(AcquisitionRequestValidationError::InvalidGameSelection);
        }
        let platforms = non_blank_values(draft.platforms);
        if platforms.is_empty() && !draft.games.fixes_platforms_explicitly() {
            return Err(AcquisitionRequestValidationError::MissingPlatforms);
        }

        let regions = non_blank_values(draft.regions);
        let languages = non_blank_values(draft.languages);
        let quality = draft
            .quality
            .map(QualityRequirements::normalized)
            .filter(|quality| quality != &QualityRequirements::default());

        Ok(Self {
            sources,
            platforms,
            games: draft.games,
            regions,
            languages,
            asset_types: draft.asset_types,
            quality,
            retention: draft.retention,
            limits: draft.limits,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetType {
    BoxFront,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SourceId(String);

impl SourceId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for SourceId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for SourceId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseAssertionField {
    Title,
    Region,
    Revision,
    Identifier,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseAssertion {
    pub source_id: SourceId,
    pub source_location: String,
    pub field: ReleaseAssertionField,
    pub qualifier: Option<String>,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceReleaseRecord {
    pub game_title: String,
    pub platform: String,
    pub region: String,
    pub revision: Option<String>,
    pub edition_name: String,
    pub assertions: Vec<ReleaseAssertion>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportedReleaseEdition {
    pub game_id: i64,
    pub release_edition_id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectorCapabilities {
    pub asset_types: Vec<AssetType>,
    pub direct_media_download: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetCandidate {
    pub game_title: String,
    pub platform: String,
    pub region: String,
    pub edition_name: String,
    pub asset_type: AssetType,
    pub source_id: SourceId,
    pub source_asset_label: Option<String>,
    pub source_url: String,
    pub original_filename: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchingPolicy {
    pub high_confidence_threshold: u8,
    pub medium_confidence_threshold: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchSignal {
    Title,
    Platform,
    Region,
    Edition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchConfidence {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchEvidence {
    pub signal: MatchSignal,
    pub candidate_value: String,
    pub release_value: String,
    pub score_delta: i16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetCandidateMatch {
    pub release_edition_id: Option<i64>,
    pub score: u8,
    pub confidence: MatchConfidence,
    pub evidence: Vec<MatchEvidence>,
}

impl AssetCandidateMatch {
    pub fn auto_link_release_edition_id(&self) -> Option<i64> {
        (self.confidence == MatchConfidence::High)
            .then_some(self.release_edition_id)
            .flatten()
    }
}

pub fn match_asset_candidate_to_release(
    candidate: &AssetCandidate,
    releases: &[LibraryEntry],
    policy: MatchingPolicy,
) -> AssetCandidateMatch {
    let mut scored_releases = releases
        .iter()
        .map(|release| {
            let evidence = vec![
                exact_match_evidence(
                    MatchSignal::Title,
                    &candidate.game_title,
                    &release.game_title,
                    50,
                ),
                exact_match_evidence(
                    MatchSignal::Platform,
                    &candidate.platform,
                    &release.platform,
                    30,
                ),
                exact_match_evidence(MatchSignal::Region, &candidate.region, &release.region, 15),
                exact_match_evidence(
                    MatchSignal::Edition,
                    &candidate.edition_name,
                    &release.edition_name,
                    5,
                ),
            ];
            let score = evidence
                .iter()
                .map(|evidence| evidence.score_delta)
                .sum::<i16>()
                .clamp(0, 100) as u8;
            (release.release_edition_id, score, evidence)
        })
        .collect::<Vec<_>>();

    scored_releases.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));

    let Some((release_edition_id, score, evidence)) = scored_releases.first().cloned() else {
        return AssetCandidateMatch {
            release_edition_id: None,
            score: 0,
            confidence: MatchConfidence::Low,
            evidence: Vec::new(),
        };
    };

    let ambiguous_best_score = scored_releases
        .get(1)
        .is_some_and(|candidate| candidate.1 == score);
    let has_material_conflict = evidence.iter().any(|evidence| {
        evidence.score_delta < 0
            && matches!(evidence.signal, MatchSignal::Region | MatchSignal::Edition)
    });
    let confidence = if (ambiguous_best_score || has_material_conflict)
        && score >= policy.medium_confidence_threshold
    {
        MatchConfidence::Medium
    } else if score >= policy.high_confidence_threshold {
        MatchConfidence::High
    } else if score >= policy.medium_confidence_threshold {
        MatchConfidence::Medium
    } else {
        MatchConfidence::Low
    };

    AssetCandidateMatch {
        release_edition_id: Some(release_edition_id),
        score,
        confidence,
        evidence,
    }
}

fn exact_match_evidence(
    signal: MatchSignal,
    candidate_value: &str,
    release_value: &str,
    score: i16,
) -> MatchEvidence {
    let score_delta = if is_missing_match_value(signal, candidate_value)
        || is_missing_match_value(signal, release_value)
    {
        0
    } else if candidate_value.trim().to_lowercase() == release_value.trim().to_lowercase() {
        score
    } else {
        -score
    };
    MatchEvidence {
        signal,
        candidate_value: candidate_value.to_owned(),
        release_value: release_value.to_owned(),
        score_delta,
    }
}

fn is_missing_match_value(signal: MatchSignal, value: &str) -> bool {
    let normalized = value.trim().to_lowercase();
    normalized.is_empty()
        || matches!(
            (signal, normalized.as_str()),
            (MatchSignal::Region, "unknown") | (MatchSignal::Edition, "unspecified")
        )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredObject {
    pub hash: String,
    pub byte_len: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistAsset {
    pub existing_game_id: Option<i64>,
    pub existing_release_edition_id: Option<i64>,
    pub game_title: String,
    pub platform: String,
    pub region: String,
    pub edition_name: String,
    pub asset_type: AssetType,
    pub object_hash: String,
    pub byte_len: u64,
    pub original_filename: String,
    pub source_id: SourceId,
    pub source_asset_label: Option<String>,
    pub source_location: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportedAsset {
    pub game_id: i64,
    pub release_edition_id: i64,
    pub asset_id: i64,
    pub object_hash: String,
    pub byte_len: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetProvenance {
    pub source_id: SourceId,
    pub source_asset_label: Option<String>,
    pub source_location: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryAsset {
    pub asset_id: i64,
    pub asset_type: AssetType,
    pub object_hash: String,
    pub byte_len: u64,
    pub original_filename: String,
    pub provenance: Vec<AssetProvenance>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryEntry {
    pub game_id: i64,
    pub game_title: String,
    pub release_edition_id: i64,
    pub platform: String,
    pub region: String,
    pub edition_name: String,
    pub assertions: Vec<ReleaseAssertion>,
    pub assets: Vec<LibraryAsset>,
}
