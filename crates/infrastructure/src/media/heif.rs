use super::boxes::{BoxHeader, BoxHeaderRead, BoxHeaderReader};

/// Largest `ftyp` and `meta` boxes kept. Item properties sit in the `meta` box, which stays
/// small since media data lives in other boxes.
const MAX_HEIF_FILE_TYPE_BOX: u64 = 4 * 1024;
const MAX_HEIF_META_BOX: u64 = 1024 * 1024;

/// Keeps the `ftyp` and `meta` boxes of a HEIF image while its bytes stream past, so its image
/// spatial extents are read wherever other boxes push the `meta` box.
#[derive(Default)]
pub(super) struct HeifPropertyScanner {
    state: HeifState,
    headers: BoxHeaderReader,
    /// The `ftyp` box, then the `meta` box once reached, headers included.
    boxes: Vec<u8>,
}

#[derive(Default, Clone, Copy)]
enum HeifState {
    #[default]
    BoxHeader,
    Keep {
        remaining: u64,
        meta: bool,
    },
    Skip(u64),
    Done,
    Stopped,
}

impl HeifPropertyScanner {
    pub(super) fn update(&mut self, mut chunk: &[u8]) {
        while !chunk.is_empty() {
            let taken = match self.state {
                HeifState::BoxHeader => {
                    let (taken, header) = self.headers.read(chunk);
                    match header {
                        BoxHeaderRead::Incomplete => {}
                        BoxHeaderRead::Invalid => self.state = HeifState::Stopped,
                        BoxHeaderRead::Complete(header) => self.state = self.start_box(&header),
                    }
                    taken
                }
                HeifState::Keep { remaining, meta } => {
                    let taken = remaining.min(chunk.len() as u64);
                    self.boxes.extend_from_slice(&chunk[..taken as usize]);
                    self.state = match remaining - taken {
                        0 if meta => HeifState::Done,
                        0 => HeifState::BoxHeader,
                        remaining => HeifState::Keep { remaining, meta },
                    };
                    taken as usize
                }
                HeifState::Skip(remaining) => {
                    let skipped = remaining.min(chunk.len() as u64);
                    self.state = match remaining - skipped {
                        0 => HeifState::BoxHeader,
                        remaining => HeifState::Skip(remaining),
                    };
                    skipped as usize
                }
                HeifState::Done | HeifState::Stopped => return,
            };
            chunk = &chunk[taken..];
        }
    }

    /// Keeps the leading `ftyp` box and the `meta` box, and skips the boxes between them.
    fn start_box(&mut self, header: &BoxHeader) -> HeifState {
        let first = self.boxes.is_empty();
        let (keep, meta) = match (&header.box_type, header.content_len) {
            (b"ftyp", Some(len)) if first && len <= MAX_HEIF_FILE_TYPE_BOX => (len, false),
            // A HEIF image starts with its file type box.
            _ if first => return HeifState::Stopped,
            (b"meta", Some(len)) if len <= MAX_HEIF_META_BOX => (len, true),
            (b"meta", _) | (_, None) => return HeifState::Stopped,
            (_, Some(len)) => return HeifState::Skip(len),
        };
        // Both limits keep the box length within a compact header.
        self.boxes
            .extend_from_slice(&((keep + 8) as u32).to_be_bytes());
        self.boxes.extend_from_slice(&header.box_type);
        HeifState::Keep {
            remaining: keep,
            meta,
        }
    }

    pub(super) fn size(&self) -> Option<(u32, u32)> {
        if !matches!(self.state, HeifState::Done) {
            return None;
        }
        let size = imagesize::blob_size(&self.boxes).ok()?;
        Some((
            u32::try_from(size.width).ok()?,
            u32::try_from(size.height).ok()?,
        ))
        .filter(|(width, height)| *width > 0 && *height > 0)
    }
}
