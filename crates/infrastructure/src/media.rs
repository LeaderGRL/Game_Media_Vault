use game_media_vault_domain::MediaInfo;
use imagesize::{Compression, ImageType};

/// Bytes read from the start of an original to identify it. JPEG frame headers beyond them are
/// found by `JpegFrameScanner` while the bytes stream past.
const MEDIA_HEADER_BYTES: usize = 256 * 1024;

/// Identifies an original from its first bytes: its media type and, for images whose header
/// fits in `header`, their pixel size.
pub fn inspect_media(header: &[u8]) -> MediaInfo {
    if header.starts_with(b"%PDF-") {
        return MediaInfo {
            media_type: "application/pdf".to_owned(),
            width: None,
            height: None,
        };
    }
    let Some(media_type) = imagesize::image_type(header)
        .ok()
        .and_then(image_media_type)
    else {
        return MediaInfo::unknown();
    };
    let size = imagesize::blob_size(header).ok();
    MediaInfo {
        media_type: media_type.to_owned(),
        width: size.and_then(|size| u32::try_from(size.width).ok()),
        height: size.and_then(|size| u32::try_from(size.height).ok()),
    }
}

fn image_media_type(image_type: ImageType) -> Option<&'static str> {
    match image_type {
        ImageType::Png => Some("image/png"),
        ImageType::Jpeg => Some("image/jpeg"),
        ImageType::Gif => Some("image/gif"),
        ImageType::Webp => Some("image/webp"),
        ImageType::Bmp => Some("image/bmp"),
        ImageType::Tiff => Some("image/tiff"),
        ImageType::Jxl => Some("image/jxl"),
        ImageType::Ico => Some("image/x-icon"),
        ImageType::Heif(Compression::Av1) => Some("image/avif"),
        ImageType::Heif(Compression::Hevc) => Some("image/heic"),
        // Texture and editing formats are kept as opaque originals.
        _ => None,
    }
}

/// Follows a JPEG's marker segments while its bytes stream past, so the frame header is found
/// wherever metadata segments put it without keeping those segments in memory.
#[derive(Default)]
pub(crate) struct JpegFrameScanner {
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
    pub(crate) fn update(&mut self, mut bytes: &[u8]) {
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
    pub(crate) fn size(&self) -> Option<(u32, u32)> {
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

/// Inspects an original while it streams: it keeps a bounded prefix for format detection and
/// follows JPEG segments, the first TIFF image file directory and JPEG XL container boxes past
/// it.
#[derive(Default)]
pub(crate) struct MediaInspector {
    header: Vec<u8>,
    jpeg: JpegFrameScanner,
    tiff: TiffDirectoryScanner,
    jxl: JxlCodestreamScanner,
}

impl MediaInspector {
    pub(crate) fn update(&mut self, chunk: &[u8]) {
        let room = MEDIA_HEADER_BYTES.saturating_sub(self.header.len());
        self.header
            .extend_from_slice(&chunk[..chunk.len().min(room)]);
        self.jpeg.update(chunk);
        self.tiff.update(chunk);
        self.jxl.update(chunk);
    }

    pub(crate) fn finish(self) -> MediaInfo {
        let mut media = inspect_media(&self.header);
        let streamed_size = match media.media_type.as_str() {
            "image/jpeg" => self.jpeg.size(),
            "image/tiff" if media.width.is_none() => self.tiff.size(),
            "image/jxl" if media.width.is_none() => self.jxl.size(),
            _ => None,
        };
        if let Some((width, height)) = streamed_size {
            media.width = Some(width);
            media.height = Some(height);
        }
        media
    }
}

const JXL_CONTAINER_SIGNATURE: &[u8] = b"\0\0\0\x0cJXL \r\n\x87\n";

/// Codestream bytes that hold a JPEG XL size header whatever its encoding.
const JXL_SIZE_HEADER_BYTES: usize = 16;

/// Follows the boxes of a JPEG XL container while its bytes stream past and keeps the start of
/// its codestream, which metadata boxes may push past the inspected prefix.
#[derive(Default)]
struct JxlCodestreamScanner {
    state: JxlState,
    /// Bytes of the structure being read: the signature, a box header or a part index.
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
    fn update(&mut self, mut chunk: &[u8]) {
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
                let mut taken = self.fill(chunk, 8);
                // A size of 1 announces a 64-bit size after the type.
                let large = self.pending.len() >= 8 && self.pending[..4] == [0, 0, 0, 1];
                if large {
                    taken += self.fill(&chunk[taken..], 16);
                }
                if self.pending.len() < if large { 16 } else { 8 } {
                    return (JxlState::BoxHeader, taken);
                }
                let state = self.start_box();
                self.pending.clear();
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

    /// Reads a complete box header from `pending` and starts its content.
    fn start_box(&self) -> JxlState {
        let size = u32::from_be_bytes([
            self.pending[0],
            self.pending[1],
            self.pending[2],
            self.pending[3],
        ]);
        let (header_len, box_len) = match size {
            0 => (8, None),
            1 => {
                let mut large = [0; 8];
                large.copy_from_slice(&self.pending[8..16]);
                (16, Some(u64::from_be_bytes(large)))
            }
            size => (8, Some(u64::from(size))),
        };
        // `None` means the box runs to the end of the file.
        let content = match box_len {
            Some(len) if len < header_len => return JxlState::Stopped,
            len => len.map(|len| len - header_len),
        };
        match (&self.pending[4..8], content) {
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

    /// Appends bytes of `chunk` to `pending` until it holds `len` bytes; returns those taken.
    fn fill(&mut self, chunk: &[u8], len: usize) -> usize {
        let taken = len.saturating_sub(self.pending.len()).min(chunk.len());
        self.pending.extend_from_slice(&chunk[..taken]);
        taken
    }

    fn size(&self) -> Option<(u32, u32)> {
        let size = imagesize::blob_size(&self.codestream).ok()?;
        Some((
            u32::try_from(size.width).ok()?,
            u32::try_from(size.height).ok()?,
        ))
        .filter(|(width, height)| *width > 0 && *height > 0)
    }
}

/// Entries read from the first TIFF image file directory at most; width and height come first
/// in practice since entries are sorted by tag.
const MAX_TIFF_DIRECTORY_ENTRIES: usize = 1024;

/// Reads the width and height tags of a TIFF's first image file directory, which the header may
/// place anywhere in the file, keeping only that directory.
#[derive(Default)]
struct TiffDirectoryScanner {
    offset: u64,
    header: Vec<u8>,
    little_endian: bool,
    directory_offset: Option<u64>,
    directory: Vec<u8>,
    size: Option<(u32, u32)>,
    stopped: bool,
}

impl TiffDirectoryScanner {
    fn update(&mut self, chunk: &[u8]) {
        let chunk_offset = self.offset;
        self.offset += chunk.len() as u64;
        if self.stopped || self.size.is_some() {
            return;
        }
        if self.directory_offset.is_none() {
            let needed = 8 - self.header.len();
            self.header
                .extend_from_slice(&chunk[..chunk.len().min(needed)]);
            if self.header.len() < 8 {
                return;
            }
            self.little_endian = match &self.header[..4] {
                b"II*\0" => true,
                b"MM\0*" => false,
                _ => {
                    self.stopped = true;
                    return;
                }
            };
            self.directory_offset = Some(u64::from(self.u32_at(&self.header, 4)));
        }
        let Some(directory_offset) = self.directory_offset else {
            return;
        };
        // Copy the part of this chunk that falls inside the directory; its length is only known
        // once the entry count is read, so copy again until nothing more is wanted.
        loop {
            let next = directory_offset + self.directory.len() as u64;
            let wanted_end = directory_offset + self.directory_len() as u64;
            let start = next.max(chunk_offset);
            let end = wanted_end.min(self.offset);
            if start >= end {
                break;
            }
            let from = (start - chunk_offset) as usize;
            let to = (end - chunk_offset) as usize;
            self.directory.extend_from_slice(&chunk[from..to]);
        }
        if self.directory.len() >= 2 && self.directory.len() >= self.directory_len() {
            self.read_directory();
        }
    }

    /// Bytes of the directory known so far: its entry count, then its entries.
    fn directory_len(&self) -> usize {
        if self.directory.len() < 2 {
            return 2;
        }
        let entries = usize::from(self.u16_at(&self.directory, 0)).min(MAX_TIFF_DIRECTORY_ENTRIES);
        2 + entries * 12
    }

    fn read_directory(&mut self) {
        let entries = (self.directory.len() - 2) / 12;
        let (mut width, mut height) = (None, None);
        for entry in 0..entries {
            let at = 2 + entry * 12;
            let value = match self.u16_at(&self.directory, at + 2) {
                3 => Some(u32::from(self.u16_at(&self.directory, at + 8))),
                4 => Some(self.u32_at(&self.directory, at + 8)),
                _ => None,
            };
            match self.u16_at(&self.directory, at) {
                256 => width = value,
                257 => height = value,
                _ => {}
            }
        }
        self.size = width.zip(height).filter(|(w, h)| *w > 0 && *h > 0);
        self.stopped = true;
    }

    fn size(&self) -> Option<(u32, u32)> {
        self.size
    }

    fn u16_at(&self, bytes: &[u8], at: usize) -> u16 {
        let pair = [bytes[at], bytes[at + 1]];
        if self.little_endian {
            u16::from_le_bytes(pair)
        } else {
            u16::from_be_bytes(pair)
        }
    }

    fn u32_at(&self, bytes: &[u8], at: usize) -> u32 {
        let quad = [bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]];
        if self.little_endian {
            u32::from_le_bytes(quad)
        } else {
            u32::from_be_bytes(quad)
        }
    }
}
