use std::io::Cursor;
use std::time::{Duration, UNIX_EPOCH};

use image::AnimationDecoder;

use crate::loader::LoadedImage;
use crate::media::MediaKind;

/// A fully decoded animation (GIF or APNG): all frames composited into RGBA,
/// with per-frame display delays and the loop count.
pub struct Anim {
    pub frames: Vec<LoadedImage>,
    /// Display time of each frame in milliseconds.
    pub delays_ms: Vec<f64>,
    /// Number of times to repeat after the first play; 0 means infinite.
    pub loop_count: u32,
}

/// Decode an animated image from raw bytes. Only handles GIF and APNG; returns
/// `None` for anything else.
pub fn decode(bytes: &[u8], ext: &str) -> Option<Anim> {
    let fmt = image::guess_format(bytes).ok()?;
    let cursor = Cursor::new(bytes);
    match fmt {
        image::ImageFormat::Gif => {
            let dec = image::codecs::gif::GifDecoder::new(cursor).ok()?;
            let loops = match dec.loop_count() {
                image::metadata::LoopCount::Infinite => 0,
                image::metadata::LoopCount::Finite(n) => n.get(),
            };
            let frames = dec.into_frames().collect::<Result<Vec<_>, _>>().ok()?;
            build(frames, loops)
        }
        image::ImageFormat::Png if ext.eq_ignore_ascii_case("apng") => {
            let dec = image::codecs::png::PngDecoder::new(cursor).ok()?.apng().ok()?;
            let loops = match dec.loop_count() {
                image::metadata::LoopCount::Infinite => 0,
                image::metadata::LoopCount::Finite(n) => n.get(),
            };
            let frames = dec.into_frames().collect::<Result<Vec<_>, _>>().ok()?;
            build(frames, loops)
        }
        _ => None,
    }
}

fn build(frames: Vec<image::Frame>, loop_count: u32) -> Option<Anim> {
    if frames.is_empty() {
        return None;
    }
    let mut anim = Anim {
        frames: Vec::with_capacity(frames.len()),
        delays_ms: Vec::with_capacity(frames.len()),
        loop_count,
    };
    for f in &frames {
        let (w, h) = f.buffer().dimensions();
        anim.frames.push(LoadedImage {
            width: w as i32,
            height: h as i32,
            pixels: f.buffer().as_raw().clone(),
            kind: MediaKind::Animated,
        });
        let ms = Duration::from(f.delay()).as_millis() as f64;
        anim.delays_ms.push(if ms <= 0.0 { 40.0 } else { ms });
    }
    Some(anim)
}

/// Playback state for an animated image, including which frame is showing and
/// how long until the next one. Uses a countdown (per-frame delay) model so that
/// manual frame stepping, looping and finite repeat counts all compose cleanly.
pub struct Player {
    pub anim: Anim,
    /// Index of the currently displayed frame.
    pub current: usize,
    /// Milliseconds remaining before advancing to the next frame.
    pub remaining_ms: f64,
    /// Number of remaining repeats after the first play; -1 means infinite.
    pub loops_remaining: i64,
    pub playing: bool,
    pub done: bool,
    /// When synced, playback is anchored to the wall clock (`sync_restart`)
    /// instead of the incremental ticker, like opti's `s` sync for GIF/APNG.
    pub synced: bool,
    pub sync_anchor_ms: u64,
}

impl Player {
    pub fn start(anim: Anim) -> Player {
        let loops_remaining = if anim.loop_count == 0 {
            -1
        } else {
            anim.loop_count as i64
        };
        let remaining_ms = anim.delays_ms.first().copied().unwrap_or(100.0);
        Player {
            anim,
            current: 0,
            remaining_ms,
            loops_remaining,
            playing: true,
            done: false,
            synced: false,
            sync_anchor_ms: 0,
        }
    }

    pub fn frame(&self) -> &LoadedImage {
        &self.anim.frames[self.current]
    }

    /// Jump `delta` frames forward (or backward) and restart the countdown for
    /// the new frame, restoring the repeat budget.
    pub fn step(&mut self, delta: i64) {
        let n = self.anim.frames.len() as isize;
        self.current = (self.current as isize + delta as isize).rem_euclid(n) as usize;
        self.remaining_ms = self.anim.delays_ms[self.current];
        self.loops_remaining = if self.anim.loop_count == 0 {
            -1
        } else {
            self.anim.loop_count as i64
        };
        self.playing = true;
        self.done = false;
        self.synced = false;
    }

    /// Return to the first frame and start playing again.
    pub fn reset(&mut self) {
        self.current = 0;
        self.remaining_ms = self.anim.delays_ms[0];
        self.loops_remaining = if self.anim.loop_count == 0 {
            -1
        } else {
            self.anim.loop_count as i64
        };
        self.playing = true;
        self.done = false;
        self.synced = false;
    }

    /// Restart playback anchored to the wall clock (`s` sync): from now on
    /// `advance` maps wall time into the frame sequence so the animation stays
    /// aligned with real time regardless of ticker drift.
    pub fn sync_restart(&mut self) {
        self.reset();
        self.synced = true;
        self.sync_anchor_ms = now_ms();
    }

    fn total_ms(&self) -> f64 {
        self.anim.delays_ms.iter().sum()
    }

    fn frame_at(&self, t_ms: f64) -> usize {
        let mut acc = 0.0;
        for (i, d) in self.anim.delays_ms.iter().enumerate() {
            acc += *d;
            if t_ms < acc {
                return i;
            }
        }
        self.anim.frames.len() - 1
    }

    /// Advance playback by `tick_ms`. Returns `true` if the displayed frame
    /// changed.
    pub fn advance(&mut self, tick_ms: f64) -> bool {
        if !self.playing || self.done || self.anim.frames.len() < 2 {
            return false;
        }
        // When synced to the wall clock, derive the frame from elapsed time.
        if self.synced {
            let total = self.total_ms();
            if total <= 0.0 {
                return false;
            }
            let now = now_ms();
            let elapsed =
                ((now as i128).wrapping_sub(self.sync_anchor_ms as i128)).rem_euclid(total as i128);
            let target = self.frame_at(elapsed as f64);
            if target != self.current {
                self.current = target;
                self.remaining_ms = self.anim.delays_ms[self.current];
                return true;
            }
            return false;
        }
        let n = self.anim.frames.len();
        self.remaining_ms -= tick_ms;
        let mut changed = false;
        while self.remaining_ms <= 0.0 && !self.done {
            // Detect a wrap from the last frame back to the first.
            if self.current == n - 1 && self.loops_remaining >= 0 {
                if self.loops_remaining <= 0 {
                    // No repeats left: stop on the final frame.
                    self.done = true;
                    self.playing = false;
                    self.remaining_ms = 0.0;
                    break;
                }
                self.loops_remaining -= 1;
            }
            self.current += 1;
            if self.current >= n {
                self.current = 0;
            }
            self.remaining_ms += self.anim.delays_ms[self.current];
            changed = true;
        }
        if self.done {
            self.current = n - 1;
        }
        changed
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_gif() -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc =
                image::codecs::gif::GifEncoder::new_with_speed(&mut out, 10);
            enc.set_repeat(image::codecs::gif::Repeat::Infinite)
                .unwrap();
            for color in [(255u8, 0u8, 0u8), (0u8, 255u8, 0u8)] {
                let img = image::RgbaImage::from_pixel(
                    4,
                    4,
                    image::Rgba([color.0, color.1, 0, 255]),
                );
                let frame = image::Frame::from_parts(
                    img,
                    0,
                    0,
                    image::Delay::from_numer_denom_ms(50, 1),
                );
                enc.encode_frame(frame).unwrap();
            }
            drop(enc);
        }
        out
    }

    #[test]
    fn decodes_animated_gif() {
        let bytes = make_gif();
        assert!(bytes.len() > 0);
        let anim = decode(&bytes, "gif").expect("gif should decode");
        assert_eq!(anim.frames.len(), 2);
        assert_eq!(anim.frames[0].width, 4);
        assert_eq!(anim.frames[0].height, 4);
        assert_eq!(anim.delays_ms, vec![50.0, 50.0]);
        assert_eq!(anim.loop_count, 0); // infinite
    }

    #[test]
    fn player_advances_and_steps_frames() {
        let mut player = Player::start(decode(&make_gif(), "gif").unwrap());
        assert_eq!(player.current, 0);
        assert!(player.advance(50.0));
        assert_eq!(player.current, 1);
        // A tick smaller than the delay does not move the frame.
        assert!(!player.advance(10.0));
        // Wraps around modulo the frame count.
        assert!(player.advance(50.0));
        assert_eq!(player.current, 0);
        // Frame stepping restarts the countdown on the new frame.
        player.step(1);
        assert_eq!(player.current, 1);
        player.step(-1);
        assert_eq!(player.current, 0);
    }
}
