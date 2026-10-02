/// Reads the dimensions of a portable anymap header while its bytes stream past, through
/// comments of any length.
#[derive(Default)]
pub(super) struct PnmHeaderScanner {
    state: PnmState,
    width: u32,
}

#[derive(Default, Clone, Copy)]
enum PnmState {
    #[default]
    Magic,
    MagicKind,
    /// The magic number is followed by whitespace or a comment.
    AfterMagic,
    /// Whitespace before the width (field 0) or the height (field 1).
    Separator(usize),
    /// A comment, up to the end of its line, before a field.
    Comment(usize),
    Number(usize, u32),
    Found(u32, u32),
    Stopped,
}

impl PnmHeaderScanner {
    pub(super) fn update(&mut self, mut chunk: &[u8]) {
        while let Some((&byte, rest)) = chunk.split_first() {
            if let PnmState::Comment(field) = self.state {
                // Skip the comment to its line end in one step.
                match chunk.iter().position(|byte| matches!(byte, b'\n' | b'\r')) {
                    Some(end) => {
                        self.state = PnmState::Separator(field);
                        chunk = &chunk[end + 1..];
                    }
                    None => return,
                }
                continue;
            }
            self.state = self.next(byte);
            if matches!(self.state, PnmState::Found(..) | PnmState::Stopped) {
                return;
            }
            chunk = rest;
        }
    }

    fn next(&mut self, byte: u8) -> PnmState {
        let space = byte.is_ascii_whitespace();
        match (self.state, byte) {
            (PnmState::Magic, b'P') => PnmState::MagicKind,
            (PnmState::MagicKind, b'1'..=b'6') => PnmState::AfterMagic,
            (PnmState::AfterMagic, b'#') => PnmState::Comment(0),
            (PnmState::Separator(field), b'#') => PnmState::Comment(field),
            (PnmState::AfterMagic, _) if space => PnmState::Separator(0),
            (PnmState::Separator(field), _) if space => PnmState::Separator(field),
            (PnmState::Separator(field), b'0'..=b'9') => {
                PnmState::Number(field, u32::from(byte - b'0'))
            }
            (PnmState::Number(field, value), b'0'..=b'9') => value
                .checked_mul(10)
                .and_then(|value| value.checked_add(u32::from(byte - b'0')))
                .map_or(PnmState::Stopped, |value| PnmState::Number(field, value)),
            (PnmState::Number(0, width), _) if space || byte == b'#' => {
                self.width = width;
                if space {
                    PnmState::Separator(1)
                } else {
                    PnmState::Comment(1)
                }
            }
            (PnmState::Number(_, height), _) if space || byte == b'#' => {
                PnmState::Found(self.width, height)
            }
            _ => PnmState::Stopped,
        }
    }

    pub(super) fn size(&self) -> Option<(u32, u32)> {
        match self.state {
            PnmState::Found(width, height) if width > 0 && height > 0 => Some((width, height)),
            _ => None,
        }
    }
}
