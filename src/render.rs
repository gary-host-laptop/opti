use cairo::{Context, Format, ImageSurface, Operator};

use crate::loader::LoadedImage;
use crate::state::ViewState;

/// Radius of the four rounded window corners, in device pixels. Must match the
/// `border-radius` set on `window` in the app CSS.
const WINDOW_RADIUS: f64 = 12.0;

/// Clip the current Cairo context so subsequent drawing stays inside a
/// rectangle whose four corners are rounded to `WINDOW_RADIUS`. The clip keeps
/// the image/overlays from painting over the transparent corner pixels.
pub fn clip_window(ctx: &Context, width: f64, height: f64) {
    use std::f64::consts::{FRAC_PI_2, PI};
    let r = WINDOW_RADIUS.min(width / 2.0).min(height / 2.0);
    let _ = ctx.save();
    ctx.new_path();
    ctx.move_to(0.0, r);
    // top-left corner: from angle PI (left) to 3PI/2 (top)
    ctx.arc(r, r, r, PI, 3.0 * FRAC_PI_2);
    ctx.line_to(width - r, 0.0);
    // top-right corner: from angle 3PI/2 (top) to 2PI (right)
    ctx.arc(width - r, r, r, 3.0 * FRAC_PI_2, 2.0 * PI);
    ctx.line_to(width, height - r);
    // bottom-right corner: from angle 0 (right) to PI/2 (bottom)
    ctx.arc(width - r, height - r, r, 0.0, FRAC_PI_2);
    ctx.line_to(r, height);
    // bottom-left corner: from angle PI/2 (bottom) to PI (left)
    ctx.arc(r, height - r, r, FRAC_PI_2, PI);
    ctx.close_path();
    ctx.clip();
    let _ = ctx.restore();
}


/// Build a Cairo image surface from the raw RGBA pixels of a loaded image.
pub fn pixels_to_surface(img: &LoadedImage) -> ImageSurface {
    let mut surface = ImageSurface::create(Format::ARgb32, img.width, img.height)
        .expect("create cairo surface");
    {
        let stride = surface.stride() as usize;
        let mut data = surface.data().expect("surface data");
        for y in 0..img.height as usize {
            let row_src = &img.pixels[y * (img.width as usize) * 4..];
            let row_dst = &mut data[y * stride..];
            for x in 0..img.width as usize {
                let s = x * 4;
                // Cairo ARGB32 is native-endian 0xAARRGGBB
                let a = row_src[s + 3];
                let r = row_src[s];
                let g = row_src[s + 1];
                let b = row_src[s + 2];
                let px = ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
                row_dst[x * 4..x * 4 + 4].copy_from_slice(&px.to_le_bytes());
            }
        }
    }
    surface.flush();
    surface
}

/// Draw the loaded image with all view transforms applied, centered in the
/// widget of `width`x`height`.
pub fn draw_image(
    cr: &Context,
    state: &ViewState,
    img: &LoadedImage,
    width: f64,
    height: f64,
) {
    if img.width == 0 || img.height == 0 {
        return;
    }

    let img_w = img.width as f64;
    let img_h = img.height as f64;
    let surface = pixels_to_surface(img);

    let _ = cr.save();
    cr.set_antialias(cairo::Antialias::Default);

    // Clear to transparent
    cr.set_operator(Operator::Source);
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.0);
    let _ = cr.paint();

    // Composite with an opaque source image, using Source so we don't blend
    // with the clear background within fragile transparent windows.
    cr.set_operator(Operator::Source);

    // Center origin
    cr.translate(width / 2.0, height / 2.0);

    // Flip
    if state.flip_x {
        cr.scale(-1.0, 1.0);
    }
    if state.flip_y {
        cr.scale(1.0, -1.0);
    }

    // Rotate (degrees)
    if state.angle != 0.0 {
        cr.rotate(state.angle.to_radians());
    }

    // Pan (in image units)
    cr.translate(state.pan_x, state.pan_y);

    // Zoom
    cr.scale(state.zoom, state.zoom);

    // Center the image geometry around origin
    cr.translate(-img_w / 2.0, -img_h / 2.0);

    // Nearest (pixelated) when zoomed in, smooth otherwise
    let filter = if state.zoom > 1.0 {
        cairo::Filter::Nearest
    } else {
        cairo::Filter::Good
    };

    let pattern = cairo::SurfacePattern::create(&surface);
    pattern.set_filter(filter);
    let _ = cr.set_source(&pattern);
    let _ = cr.paint_with_alpha(state.opacity);

    let _ = cr.restore();
}

/// Render the filename at the bottom edge (matches the `.title` overlay).
pub fn draw_name_overlay(cr: &Context, state: &ViewState, width: f64, height: f64) {
    use cairo::{FontSlant, FontWeight};
    if !state.show_name {
        return;
    }
    let Some(file) = state.fullpath.as_ref().and_then(|p| p.file_name()) else {
        return;
    };
    let text = file.to_string_lossy().to_string();

    let _ = cr.save();
    cr.set_operator(Operator::Over);
    let font_size = 19.0;
    cr.select_font_face("Varela Round", FontSlant::Normal, FontWeight::Normal);
    cr.set_font_size(font_size);
    let text_w = cr.text_extents(&text).map(|e| e.width()).unwrap_or(0.0);

    // Shadow for legibility
    cr.set_source_rgba(0.0, 0.0, 0.0, 1.0);
    cr.move_to((width - text_w) / 2.0 + 1.0, height - font_size - 9.0 + 1.0);
    let _ = cr.show_text(&text);
    cr.set_source_rgba(1.0, 1.0, 1.0, 1.0);
    cr.move_to((width - text_w) / 2.0, height - font_size - 9.0);
    let _ = cr.show_text(&text);
    let _ = cr.restore();
}

const HELP_LINES: &[&str] = &[
    "? help       a always-on-top",
    "p passthrough       l lock window       o reveal file",
    "n file name       r reload   shift+r restart   s sync to clock",
    "",
    "right/left next/prev file   +shift media file",
    "enter open folder   backspace parent",
    "",
    "f / +shift flip h / v",
    "move window (super+drag)   +ctrl pan image",
    "resize window (frame edges)   scroll pixel scale   +ctrl image only",
    "right-click rotate   +shift snap-rotate   +alt drag-scale",
    "+shift opacity",
    "",
    "</> seek frame   +shift video 1/60s",
    "ctrl+c copy   ctrl+alt+c data uri   ctrl+v paste",
    "space reset   shift+space reset player   ctrl+space keep size",
    "f11 fullscreen   escape close",
];

/// Render the help overlay as a translucent panel in the center.
pub fn draw_help_overlay(cr: &Context, state: &ViewState, width: f64, height: f64) {
    use cairo::{FontSlant, FontWeight};
    if !state.show_help {
        return;
    }
    let _ = cr.save();
    cr.set_operator(Operator::Over);

    // Translucent backdrop
    cr.set_source_rgba(0.063, 0.059, 0.075, 0.71);
    cr.rectangle(0.0, 0.0, width, height);
    let _ = cr.fill();

    cr.select_font_face("Varela Round", FontSlant::Normal, FontWeight::Normal);
    let font_size = 15.0;
    cr.set_font_size(font_size);

    let line_height = font_size * 1.5;
    let total_h = HELP_LINES.len() as f64 * line_height;
    let mut y = (height - total_h) / 2.0 + font_size;

    for line in HELP_LINES {
        let text_w = cr.text_extents(line).map(|e| e.width()).unwrap_or(0.0);
        cr.set_source_rgba(0.9, 0.88, 0.91, 1.0);
        cr.move_to((width - text_w) / 2.0, y);
        let _ = cr.show_text(line);
        y += line_height;
    }

    let _ = cr.restore();
}
