use gdk_pixbuf::prelude::PixbufLoaderExt;
use gdk_pixbuf::{Pixbuf, PixbufLoader};

use crate::media::MediaKind;

/// A decoded static image ready for rendering/drawing.
#[derive(Clone)]
pub struct LoadedImage {
    pub width: i32,
    pub height: i32,
    /// RGBA8 raw pixels, height-major.
    pub pixels: Vec<u8>,
    pub kind: MediaKind,
}

/// Detect media kind from file extension + magic bytes, returning the associated
/// metadata we need. For M0 we fully decode only static images; other kinds are
/// recognized so later milestones can take over.
pub fn detect(bytes: &[u8], ext: &str) -> MediaKind {
    let fmt = image::guess_format(bytes).ok().or_else(|| ext_image_format(ext));
    match fmt {
        Some(f) if is_animated_format(f, ext) => MediaKind::Animated,
        Some(f) if f == image::ImageFormat::Gif && ext != "gif" => MediaKind::Animated,
        Some(f) => MediaKind::Image(f),
        None => MediaKind::None,
    }
}

/// Video container/file extensions routed to GStreamer playback.
pub const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "m4v", "mkv", "ogv", "ogm", "webm", "avi", "3gp", "mov", "mpeg", "mpg", "ts", "m2ts",
];

/// Audio file extensions routed to GStreamer playback.
pub const AUDIO_EXTENSIONS: &[&str] = &[
    "flac", "m4a", "mp3", "ogg", "oga", "opus", "wav", "amr", "aac", "wavpack", "ac3",
];

/// Full classification used by the loader: images/animated as before, plus
/// video/audio routed to GStreamer based on the extension.
pub fn detect_media(bytes: &[u8], ext: &str) -> MediaKind {
    match detect(bytes, ext) {
        MediaKind::None => {
            let e = ext.to_lowercase();
            if VIDEO_EXTENSIONS.contains(&e.as_str()) {
                MediaKind::Video
            } else if AUDIO_EXTENSIONS.contains(&e.as_str()) {
                MediaKind::Audio
            } else {
                MediaKind::None
            }
        }
        k => k,
    }
}

fn ext_image_format(ext: &str) -> Option<image::ImageFormat> {
    match ext.to_lowercase().as_str() {
        "gif" => Some(image::ImageFormat::Gif),
        "png" | "apng" => Some(image::ImageFormat::Png),
        "jpeg" | "jpg" | "jpe" | "jfif" => Some(image::ImageFormat::Jpeg),
        "webp" => Some(image::ImageFormat::WebP),
        "bmp" => Some(image::ImageFormat::Bmp),
        "tiff" | "tif" => Some(image::ImageFormat::Tiff),
        "ico" => Some(image::ImageFormat::Ico),
        "svg" => None, // handled by pixbuf librsvg below
        _ => None,
    }
}

fn is_animated_format(fmt: image::ImageFormat, ext: &str) -> bool {
    const ANIMATED: &[image::ImageFormat] = &[
        image::ImageFormat::Gif,
        image::ImageFormat::Png, // apng lives in png
        image::ImageFormat::WebP,
    ];
    if ANIMATED.contains(&fmt) {
        // apng lives in png
        if fmt == image::ImageFormat::Gif {
            return true;
        }
        if fmt == image::ImageFormat::Png && ext.eq_ignore_ascii_case("apng") {
            return true;
        }
    }
    false
}

/// Decode a raw byte buffer into [LoadedImage]. Returns None for formats we
/// don't handle in M0 (video/audio/svg/animated handled later).
pub fn load(bytes: &[u8], ext: &str) -> Option<LoadedImage> {
    let kind = detect(bytes, ext);
    match kind {
        MediaKind::Image(_) => decode_image(bytes),
        _ => None,
    }
}

fn decode_image(bytes: &[u8]) -> Option<LoadedImage> {
    // Prefer gdk-pixbuf (handles svg via librsvg in a standard build).
    let loader = PixbufLoader::new();
    if loader.write(bytes).is_err() {
        return fallback_image(bytes);
    }
    let _ = loader.close();
    if let Some(pixbuf) = loader.pixbuf() {
        return Some(from_pixbuf(pixbuf));
    }

    fallback_image(bytes)
}

fn fallback_image(bytes: &[u8]) -> Option<LoadedImage> {
    // Fall back to the pure-rust `image` crate.
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    Some(LoadedImage {
        width: w as i32,
        height: h as i32,
        pixels: img.into_raw(),
        kind: MediaKind::Image(image::ImageFormat::Png),
    })
}

/// Encode raw RGBA pixels as PNG bytes (used for clipboard copy).
pub fn encode_png_rgba(pixels: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    use image::codecs::png::PngEncoder;
    use image::ImageEncoder;
    let mut out = Vec::new();
    PngEncoder::new(&mut out)
        .write_image(
            pixels,
            width,
            height,
            image::ExtendedColorType::Rgba8,
        )
        .ok()?;
    Some(out)
}

fn from_pixbuf(pixbuf: Pixbuf) -> LoadedImage {
    let width = pixbuf.width();
    let height = pixbuf.height();
    let n_channels = pixbuf.n_channels();
    let rowstride = pixbuf.rowstride();
    let has_alpha = pixbuf.has_alpha();
    let owned_bytes = pixbuf.pixel_bytes();
    let src: &[u8] = match &owned_bytes {
        Some(b) => b.as_ref(),
        None => &[],
    };

    let mut pixels = Vec::with_capacity((width as usize) * (height as usize) * 4);
    for y in 0..height {
        let row = &src[(y as usize) * rowstride as usize..];
        for x in 0..width {
            let i = (x as usize) * n_channels as usize;
            let (r, g, b) = (row[i], row[i + 1], row[i + 2]);
            let a = if has_alpha { row[i + 3] } else { 255 };
            pixels.extend_from_slice(&[r, g, b, a]);
        }
    }

    LoadedImage {
        width,
        height,
        pixels,
        kind: MediaKind::Image(image::ImageFormat::Png),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gif_magic_detected_as_animated() {
        let bytes = b"GIF89a\x01\x00\x01\x00\x00\x00\x00;\x00";
        assert_eq!(detect(bytes, "gif"), MediaKind::Animated);
    }

    #[test]
    fn png_magic_detected_as_regular_image() {
        let bytes = [0x89u8, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        match detect(&bytes, "png") {
            MediaKind::Image(_) => {}
            _ => panic!("png should be a single image"),
        }
    }

    #[test]
    fn png_roundtrip_encode() {
        let pixels = vec![255u8, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 0, 0, 255];
        let png = encode_png_rgba(&pixels, 2, 2).unwrap();
        assert!(png.len() > 8);
        let decoded = image::load_from_memory(&png).unwrap().into_rgba8();
        assert_eq!(decoded.dimensions(), (2, 2));
        assert_eq!(decoded.into_raw(), pixels);
    }
}


