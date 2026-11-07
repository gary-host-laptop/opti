use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;

use gstreamer as gst;
use gst::prelude::*;

use crate::settings::Settings;

pub const VOLUME_STEP: f64 = 0.05;
const SHIFT_SEEK: f64 = 1.0 / 60.0;

/// Debug logging is gated behind the `OPTI_DEBUG` env var so normal runs stay
/// quiet but verification runs can confirm playback started and frames decode.
pub fn debug_log() -> bool {
    std::env::var_os("OPTI_DEBUG").is_some()
}

/// Latest decoded video frame, written by the GStreamer streaming thread and
/// read by the main-thread ticker.
pub struct FrameState {
    pub width: i32,
    pub height: i32,
    pub pixels: Vec<u8>,
    pub fps: f64,
    /// Incremented on every new frame so consumers can cheaply detect updates.
    pub seq: u64,
}

impl FrameState {
    fn empty() -> Self {
        Self {
            width: 0,
            height: 0,
            pixels: Vec::new(),
            fps: 0.0,
            seq: 0,
        }
    }
}

/// A GStreamer playbin pipeline whose video sink is an appsink delivering raw
/// RGBA frames. Audio is played through playbin's default audi(autoaudiosink).
pub struct Playback {
    pipeline: gst::Pipeline,
    playbin: gst::Element,
    pub frame: Arc<Mutex<FrameState>>,
    pub playing: Rc<RefCell<bool>>,
    _watch: gst::bus::BusWatchGuard,
}

fn mod_i64(a: i64, m: i64) -> i64 {
    if m <= 0 {
        return 0;
    }
    ((a % m) + m) % m
}

impl Playback {
    pub fn new(uri: &str, volume: f64) -> Option<Self> {
        gst::init().ok()?;

        let playbin = gst::ElementFactory::make("playbin")
            .name("playbin")
            .property("uri", uri)
            .build()
            .ok()?;

        let rcaps: gst::Caps = "video/x-raw,format=(string)RGBA".parse().ok()?;
        let appsink = gst::ElementFactory::make("appsink")
            .name("sink")
            .property("emit-signals", true)
            .property("caps", &rcaps)
            .build()
            .ok()?;
        playbin.set_property("video-sink", &appsink);
        playbin.set_property("volume", volume.clamp(0.0, 1.0));

        let pipeline = gst::Pipeline::new();
        if pipeline.add(&playbin).is_err() {
            return None;
        }
        if playbin.sync_state_with_parent().is_err() {
            return None;
        }

        let frame = Arc::new(Mutex::new(FrameState::empty()));
        let frames_cb = frame.clone();
        let fps_of =
            |caps: &gst::CapsRef| -> f64 {
                caps.structure(0)
                    .and_then(|s| s.get::<gst::Fraction>("framerate").ok())
                    .map(|f| f.numer() as f64 / f.denom() as f64)
                    .unwrap_or(0.0)
            };
        appsink.connect("new-sample", false, move |values| {
            if let Ok(element) = values[0].get::<gst::Element>() {
                let sample: gst::Sample = element.property("last-sample");
                if let Some(buf) = sample.buffer() {
                    if let Ok(map) = buf.map_readable() {
                        let caps = sample.caps();
                        let (w, h) = caps
                            .as_ref()
                            .and_then(|c| c.structure(0))
                            .map(|s| {
                                (
                                    s.get::<i32>("width").unwrap_or(0),
                                    s.get::<i32>("height").unwrap_or(0),
                                )
                            })
                            .unwrap_or((0, 0));
                        if w > 0 && h > 0 {
                            let mut fr = frames_cb.lock().unwrap();
                            fr.width = w;
                            fr.height = h;
                            fr.pixels.clear();
                            fr.pixels.extend_from_slice(map.as_ref());
                            fr.fps = caps.map(|c| fps_of(&c)).unwrap_or(0.0);
                            fr.seq += 1;
                            if debug_log() && (fr.seq <= 3 || fr.seq % 50 == 0) {
                                eprintln!(
                                    "opti: [playback] frame seq={} {}x{}",
                                    fr.seq, fr.width, fr.height
                                );
                            }
                        }
                    }
                }
            }
            Some(gst::glib::Value::from(gst::FlowReturn::Ok))
        });

        let playing = Rc::new(RefCell::new(true));
        if let Err(e) = pipeline.set_state(gst::State::Playing) {
            eprintln!("opti: failed to start playback: {e}");
            return None;
        }
        if debug_log() {
            eprintln!("opti: [playback] started uri={uri}");
        }

        let stash = Rc::new(RefCell::new(Some(pipeline.clone())));
        let bus = pipeline.bus()?;
        let _watch = bus
            .add_watch_local(move |_b, msg| {
                match msg.view() {
                    gst::MessageView::Eos(_) => {
                        // opti's media elements use `loop`; playbin doesn't loop
                        // on its own, so seek back to the start to continue.
                        if let Some(p) = stash.borrow().as_ref() {
                            let _ = p.seek_simple(
                                gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT,
                                gst::ClockTime::from_nseconds(0),
                            );
                        }
                    }
                    gst::MessageView::Error(e) => {
                        eprintln!("opti: playback error: {e}");
                    }
                    _ => {}
                }
                gst::glib::ControlFlow::Continue
            })
            .ok()?;

        Some(Self {
            pipeline,
            playbin,
            frame,
            playing,
            _watch,
        })
    }

    pub fn stop(&self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }

    pub fn is_playing(&self) -> bool {
        *self.playing.borrow()
    }

    pub fn play(&self) {
        if self.pipeline.set_state(gst::State::Playing).is_ok() {
            *self.playing.borrow_mut() = true;
        }
    }

    pub fn pause(&self) {
        if self.pipeline.set_state(gst::State::Paused).is_ok() {
            *self.playing.borrow_mut() = false;
        }
    }

    pub fn toggle_pause(&self) {
        if self.is_playing() {
            self.pause();
        } else {
            self.play();
        }
    }

    pub fn duration_ns(&self) -> Option<u64> {
        self.pipeline
            .query_duration::<gst::ClockTime>()
            .map(|d| d.nseconds())
    }

    pub fn position_ns(&self) -> Option<u64> {
        self.pipeline
            .query_position::<gst::ClockTime>()
            .map(|p| p.nseconds())
    }

    pub fn seek_to_ns(&self, ns: u64) {
        let _ = self.pipeline.seek_simple(
            gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
            gst::ClockTime::from_nseconds(ns),
        );
    }

    /// Seek by whole frames (mirrors opti's non-shift `seekFrame`), wrapping
    /// around the duration like opti's `mod`.
    pub fn step_frames(&self, delta: i32) {
        let fps = self.frame.lock().unwrap().fps;
        let seconds = if fps > 0.0 {
            delta as f64 / fps
        } else {
            delta as f64 * SHIFT_SEEK
        };
        self.relative_seek(seconds);
    }

    /// Seek by 1/60th-second increments (mirrors opti's shift `seekFrame`).
    pub fn shift_seek(&self, delta: i32) {
        self.relative_seek(delta as f64 * SHIFT_SEEK);
    }

    fn relative_seek(&self, seconds: f64) {
        let Some(dur) = self.duration_ns() else { return };
        if dur == 0 {
            return;
        }
        let pos = self.position_ns().unwrap_or(0);
        let step = (seconds * 1e9) as i64;
        let target = mod_i64(pos as i64 + step, dur as i64) as u64;
        self.seek_to_ns(target);
    }

    /// Align the current position with the wall clock: `pos = epoch_ms % duration`
    /// and resume play, so multiple windows stay in sync (opti's `syncAnimation`).
    pub fn sync_to_epoch(&self) {
        let Some(dur) = self.duration_ns() else { return };
        if dur == 0 {
            return;
        }
        let now_ms = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let dur_ms = (dur / 1_000_000).max(1) as i128;
        let pos_ns = ((now_ms as i128) % dur_ms) as u64 * 1_000_000;
        self.seek_to_ns(pos_ns);
        self.play();
    }

    pub fn set_volume(&self, v: f64) {
        self.playbin
            .set_property("volume", v.clamp(0.0, 1.0) as f64);
    }

    pub fn get_volume(&self) -> f64 {
        self.playbin
            .property::<f64>("volume")
            .clamp(0.0, 1.0)
    }

    /// Adjust volume by `VOLUME_STEP`, persisting the new value to settings.
    pub fn nudge_volume(&self, dir: f64, settings: &mut Settings) {
        let v = (self.get_volume() + dir * VOLUME_STEP).clamp(0.0, 1.0);
        self.set_volume(v);
        settings.set_volume(v);
        settings.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mod_i64_wraps_negative() {
        assert_eq!(mod_i64(-1, 10), 9);
        assert_eq!(mod_i64(-13, 10), 7);
        assert_eq!(mod_i64(13, 10), 3);
        assert_eq!(mod_i64(0, 10), 0);
    }

    #[test]
    fn volume_clamps_to_unit_range() {
        let _ = gst::init();
        let playbin = gst::ElementFactory::make("playbin").build().unwrap();
        let player = Playback {
            pipeline: gst::Pipeline::new(),
            playbin,
            frame: Arc::new(Mutex::new(FrameState::empty())),
            playing: Rc::new(RefCell::new(true)),
            _watch: {
                let bus = gst::Bus::new();
                bus.add_watch_local(|_, _| gst::glib::ControlFlow::Continue)
                    .expect("watch")
            },
        };
        player.set_volume(1.5);
        assert_eq!(player.get_volume(), 1.0);
        player.set_volume(-0.3);
        assert_eq!(player.get_volume(), 0.0);
        player.set_volume(0.4);
        assert_eq!(player.get_volume(), 0.4);
    }
}