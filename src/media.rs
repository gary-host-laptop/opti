#[derive(Clone, Debug, PartialEq)]
pub enum MediaKind {
    Image(image::ImageFormat),
    Animated,
    Video,
    Audio,
    None,
}

impl MediaKind {
    pub fn is_animated(&self) -> bool {
        matches!(self, MediaKind::Animated)
    }
}
