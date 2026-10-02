use serde::{Deserialize, Serialize};

use crate::{MediaInfo, QualityRequirements};

/// A measurable way an original falls short of the request's quality requirements. Unknown
/// pixel sizes fall short of every size requirement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "requirement", rename_all = "snake_case")]
pub enum QualityShortfall {
    MinWidth {
        minimum: u32,
        actual: Option<u32>,
    },
    MinHeight {
        minimum: u32,
        actual: Option<u32>,
    },
    MinLongestEdge {
        minimum: u32,
        actual: Option<u32>,
    },
    MinPixelCount {
        minimum: u64,
        actual: Option<u64>,
    },
    AcceptedMediaTypes {
        accepted: Vec<String>,
        actual: String,
    },
}

impl QualityRequirements {
    /// How `media` falls short of these requirements, in declaration order; empty when it
    /// meets them all.
    pub fn shortfalls(&self, media: &MediaInfo) -> Vec<QualityShortfall> {
        let mut shortfalls = Vec::new();
        let longest_edge = media.width.zip(media.height).map(|(w, h)| w.max(h));
        let pixel_count = media
            .width
            .zip(media.height)
            .map(|(w, h)| u64::from(w) * u64::from(h));
        if let Some(minimum) = self.min_width
            && media.width.is_none_or(|width| width < minimum)
        {
            shortfalls.push(QualityShortfall::MinWidth {
                minimum,
                actual: media.width,
            });
        }
        if let Some(minimum) = self.min_height
            && media.height.is_none_or(|height| height < minimum)
        {
            shortfalls.push(QualityShortfall::MinHeight {
                minimum,
                actual: media.height,
            });
        }
        if let Some(minimum) = self.min_longest_edge
            && longest_edge.is_none_or(|edge| edge < minimum)
        {
            shortfalls.push(QualityShortfall::MinLongestEdge {
                minimum,
                actual: longest_edge,
            });
        }
        if let Some(minimum) = self.min_pixel_count
            && pixel_count.is_none_or(|count| count < minimum)
        {
            shortfalls.push(QualityShortfall::MinPixelCount {
                minimum,
                actual: pixel_count,
            });
        }
        if !self.accepted_mime_types.is_empty()
            && !self
                .accepted_mime_types
                .iter()
                .any(|accepted| accepted.eq_ignore_ascii_case(&media.media_type))
        {
            shortfalls.push(QualityShortfall::AcceptedMediaTypes {
                accepted: self.accepted_mime_types.clone(),
                actual: media.media_type.clone(),
            });
        }
        shortfalls
    }

    /// The first requirement the engine cannot apply yet, if any. Every acquired Asset is an
    /// original, so `original_only` always holds.
    pub fn unsupported_requirement(&self) -> Option<&'static str> {
        if self.max_compression_ratio.is_some() {
            Some("compression ratios cannot be measured yet")
        } else if self.min_bitrate_kbps.is_some() {
            Some("bitrates cannot be measured yet")
        } else if self.preferred_scan_type.is_some() || !self.preferred_source_priority.is_empty() {
            Some(
                "scan type and source preferences need a configurable scoring policy, which is not supported yet",
            )
        } else if self.best_available {
            Some(
                "best-available mode needs a configurable scoring policy, which is not supported yet",
            )
        } else {
            None
        }
    }
}
