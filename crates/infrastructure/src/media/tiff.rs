/// Entries read from the first TIFF image file directory at most; width and height come first
/// in practice since entries are sorted by tag.
const MAX_TIFF_DIRECTORY_ENTRIES: usize = 1024;

/// Reads the width and height tags of a TIFF's first image file directory, which the header may
/// place anywhere in the file, keeping only that directory.
#[derive(Default)]
pub(super) struct TiffDirectoryScanner {
    offset: u64,
    header: Vec<u8>,
    little_endian: bool,
    directory_offset: Option<u64>,
    directory: Vec<u8>,
    size: Option<(u32, u32)>,
    stopped: bool,
}

impl TiffDirectoryScanner {
    pub(super) fn update(&mut self, chunk: &[u8]) {
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

    pub(super) fn size(&self) -> Option<(u32, u32)> {
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
