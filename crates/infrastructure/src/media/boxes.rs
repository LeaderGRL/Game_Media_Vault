/// Type and content length of an ISO base media box; `None` lengths run to the end of the file.
pub(super) struct BoxHeader {
    pub(super) box_type: [u8; 4],
    pub(super) content_len: Option<u64>,
}

pub(super) enum BoxHeaderRead {
    Incomplete,
    Complete(BoxHeader),
    Invalid,
}

/// Reads the header of the next ISO base media box, as HEIF images and JPEG XL containers
/// use them, from bytes that arrive in chunks.
#[derive(Default)]
pub(super) struct BoxHeaderReader {
    pending: Vec<u8>,
}

impl BoxHeaderReader {
    /// Consumes header bytes from the start of `chunk`, returning how many it took.
    pub(super) fn read(&mut self, chunk: &[u8]) -> (usize, BoxHeaderRead) {
        let mut taken = fill(&mut self.pending, chunk, 8);
        // A size of 1 announces a 64-bit size after the type.
        let large = self.pending.len() >= 8 && self.pending[..4] == [0, 0, 0, 1];
        if large {
            taken += fill(&mut self.pending, &chunk[taken..], 16);
        }
        if self.pending.len() < if large { 16 } else { 8 } {
            return (taken, BoxHeaderRead::Incomplete);
        }
        let header = Self::parse(&self.pending);
        self.pending.clear();
        (
            taken,
            header.map_or(BoxHeaderRead::Invalid, BoxHeaderRead::Complete),
        )
    }

    fn parse(bytes: &[u8]) -> Option<BoxHeader> {
        let size = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let (header_len, box_len) = match size {
            0 => (8, None),
            1 => {
                let mut large = [0; 8];
                large.copy_from_slice(&bytes[8..16]);
                (16, Some(u64::from_be_bytes(large)))
            }
            size => (8, Some(u64::from(size))),
        };
        let content_len = match box_len {
            Some(len) if len < header_len => return None,
            len => len.map(|len| len - header_len),
        };
        Some(BoxHeader {
            box_type: [bytes[4], bytes[5], bytes[6], bytes[7]],
            content_len,
        })
    }
}

/// Appends bytes of `chunk` to `pending` until it holds `len` bytes; returns those taken.
pub(super) fn fill(pending: &mut Vec<u8>, chunk: &[u8], len: usize) -> usize {
    let taken = len.saturating_sub(pending.len()).min(chunk.len());
    pending.extend_from_slice(&chunk[..taken]);
    taken
}
