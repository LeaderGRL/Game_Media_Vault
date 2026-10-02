/// Follows a JPEG's marker segments while its bytes stream past, so the frame header is found
/// wherever metadata segments put it without keeping those segments in memory.
#[derive(Default)]
pub(super) struct JpegFrameScanner {
    state: ScanState,
}

#[derive(Default)]
enum ScanState {
    #[default]
    StartOfImage,
    StartOfImageCode,
    MarkerPrefix,
    MarkerCode,
    LengthHigh(u8),
    LengthLow(u8, u8),
    Skip(usize),
    Frame(Vec<u8>),
    Found(u32, u32),
    Stopped,
}

impl JpegFrameScanner {
    pub(super) fn update(&mut self, mut bytes: &[u8]) {
        while let Some((&byte, rest)) = bytes.split_first() {
            self.state = match std::mem::take(&mut self.state) {
                ScanState::Skip(remaining) => {
                    // Skip the segment body in one step instead of byte by byte.
                    let skipped = remaining.min(bytes.len());
                    bytes = &bytes[skipped..];
                    self.state = if remaining > skipped {
                        ScanState::Skip(remaining - skipped)
                    } else {
                        ScanState::MarkerPrefix
                    };
                    continue;
                }
                state @ (ScanState::Found(..) | ScanState::Stopped) => {
                    self.state = state;
                    return;
                }
                state => Self::next(state, byte),
            };
            bytes = rest;
        }
    }

    /// Width and height from the frame header, once found.
    pub(super) fn size(&self) -> Option<(u32, u32)> {
        match self.state {
            ScanState::Found(width, height) if width > 0 && height > 0 => Some((width, height)),
            _ => None,
        }
    }

    fn next(state: ScanState, byte: u8) -> ScanState {
        match (state, byte) {
            (ScanState::StartOfImage, 0xff) => ScanState::StartOfImageCode,
            (ScanState::StartOfImageCode, 0xd8) => ScanState::MarkerPrefix,
            (ScanState::MarkerPrefix, 0xff) => ScanState::MarkerCode,
            // Fill bytes may precede a marker code.
            (ScanState::MarkerCode, 0xff) => ScanState::MarkerCode,
            // Markers without a length: TEM and restart markers.
            (ScanState::MarkerCode, 0x01 | 0xd0..=0xd7) => ScanState::MarkerPrefix,
            (ScanState::MarkerCode, 0xd8 | 0xd9) => ScanState::Stopped,
            (ScanState::MarkerCode, marker) => ScanState::LengthHigh(marker),
            (ScanState::LengthHigh(marker), high) => ScanState::LengthLow(marker, high),
            (ScanState::LengthLow(marker, high), low) => {
                let body = usize::from(u16::from_be_bytes([high, low])).saturating_sub(2);
                if is_start_of_frame(marker) {
                    ScanState::Frame(Vec::with_capacity(5))
                } else if body == 0 {
                    ScanState::MarkerPrefix
                } else {
                    ScanState::Skip(body)
                }
            }
            // Precision, then height and width, both big-endian.
            (ScanState::Frame(mut header), byte) => {
                header.push(byte);
                if header.len() == 5 {
                    let height = u16::from_be_bytes([header[1], header[2]]);
                    let width = u16::from_be_bytes([header[3], header[4]]);
                    ScanState::Found(u32::from(width), u32::from(height))
                } else {
                    ScanState::Frame(header)
                }
            }
            _ => ScanState::Stopped,
        }
    }
}

fn is_start_of_frame(marker: u8) -> bool {
    matches!(marker, 0xc0..=0xcf) && !matches!(marker, 0xc4 | 0xc8 | 0xcc)
}
