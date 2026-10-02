use super::boxes::{BoxHeader, BoxHeaderRead, BoxHeaderReader, fill};

const JXL_CONTAINER_SIGNATURE: &[u8] = b"\0\0\0\x0cJXL \r\n\x87\n";

/// Codestream bytes that hold a JPEG XL size header whatever its encoding.
const JXL_SIZE_HEADER_BYTES: usize = 16;

/// Follows the boxes of a JPEG XL container while its bytes stream past and keeps the start of
/// its codestream, which metadata boxes may push past the inspected prefix.
#[derive(Default)]
pub(super) struct JxlCodestreamScanner {
    state: JxlState,
    headers: BoxHeaderReader,
    /// Bytes of the signature or of a part index being read.
    pending: Vec<u8>,
    codestream: Vec<u8>,
}

#[derive(Default, Clone, Copy)]
enum JxlState {
    #[default]
    Signature,
    BoxHeader,
    /// Skips the rest of a box.
    Skip(u64),
    /// Reads the index that starts a partial codestream box; `None` sizes run to the end.
    PartIndex(Option<u64>),
    Codestream {
        remaining: Option<u64>,
        last: bool,
    },
    Stopped,
}

impl JxlCodestreamScanner {
    pub(super) fn update(&mut self, mut chunk: &[u8]) {
        while !chunk.is_empty() {
            let (state, consumed) = self.next(chunk);
            self.state = state;
            chunk = &chunk[consumed..];
            if matches!(self.state, JxlState::Stopped) {
                return;
            }
        }
    }

    /// Advances over the start of `chunk`, returning the next state and the bytes consumed.
    fn next(&mut self, chunk: &[u8]) -> (JxlState, usize) {
        match self.state {
            JxlState::Signature => {
                let taken = self.fill(chunk, JXL_CONTAINER_SIGNATURE.len());
                let state = match self.pending.len() {
                    len if len < JXL_CONTAINER_SIGNATURE.len() => JxlState::Signature,
                    _ if self.pending == JXL_CONTAINER_SIGNATURE => {
                        self.pending.clear();
                        JxlState::BoxHeader
                    }
                    _ => JxlState::Stopped,
                };
                (state, taken)
            }
            JxlState::BoxHeader => {
                let (taken, header) = self.headers.read(chunk);
                let state = match header {
                    BoxHeaderRead::Incomplete => JxlState::BoxHeader,
                    BoxHeaderRead::Invalid => JxlState::Stopped,
                    BoxHeaderRead::Complete(header) => Self::start_box(header),
                };
                (state, taken)
            }
            JxlState::Skip(remaining) => {
                let skipped = remaining.min(chunk.len() as u64);
                let state = if remaining > skipped {
                    JxlState::Skip(remaining - skipped)
                } else {
                    JxlState::BoxHeader
                };
                (state, skipped as usize)
            }
            JxlState::PartIndex(remaining) => {
                let taken = self.fill(chunk, 4);
                if self.pending.len() < 4 {
                    return (JxlState::PartIndex(remaining), taken);
                }
                // The high bit marks the last part of the codestream.
                let last = self.pending[0] & 0x80 != 0;
                self.pending.clear();
                let state = match remaining {
                    Some(remaining) if remaining < 4 => JxlState::Stopped,
                    remaining => JxlState::Codestream {
                        remaining: remaining.map(|remaining| remaining - 4),
                        last,
                    },
                };
                (state, taken)
            }
            JxlState::Codestream { remaining, last } => {
                let wanted = JXL_SIZE_HEADER_BYTES - self.codestream.len();
                let available = remaining.map_or(chunk.len(), |remaining| {
                    remaining.min(chunk.len() as u64) as usize
                });
                let taken = wanted.min(available);
                self.codestream.extend_from_slice(&chunk[..taken]);
                let remaining = remaining.map(|remaining| remaining - taken as u64);
                let state = if self.codestream.len() == JXL_SIZE_HEADER_BYTES
                    || (last && remaining == Some(0))
                {
                    JxlState::Stopped
                } else if remaining == Some(0) {
                    JxlState::BoxHeader
                } else {
                    JxlState::Codestream { remaining, last }
                };
                (state, taken)
            }
            JxlState::Stopped => (JxlState::Stopped, chunk.len()),
        }
    }

    /// Starts the content of a box: the codestream is kept, other boxes are skipped.
    fn start_box(header: BoxHeader) -> JxlState {
        match (&header.box_type, header.content_len) {
            (b"jxlc", content) => JxlState::Codestream {
                remaining: content,
                last: true,
            },
            (b"jxlp", content) => JxlState::PartIndex(content),
            (_, Some(content)) => JxlState::Skip(content),
            // Nothing follows a box that runs to the end of the file.
            (_, None) => JxlState::Stopped,
        }
    }

    fn fill(&mut self, chunk: &[u8], len: usize) -> usize {
        fill(&mut self.pending, chunk, len)
    }

    pub(super) fn size(&self) -> Option<(u32, u32)> {
        let size = imagesize::blob_size(&self.codestream).ok()?;
        Some((
            u32::try_from(size.width).ok()?,
            u32::try_from(size.height).ok()?,
        ))
        .filter(|(width, height)| *width > 0 && *height > 0)
    }
}
