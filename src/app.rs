use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{gdk, glib, Application, ApplicationWindow, CssProvider, DrawingArea};

use crate::anim;
use crate::anim::Player;
use crate::loader::{self, LoadedImage};
use crate::media::MediaKind;
use crate::navigation;
use crate::network;
use crate::playback::{self, Playback, VOLUME_STEP};
use crate::render;
use crate::state::{
    Context, DragMode, ViewState,
};
use crate::xwin;
use crate::settings::Settings;

/// Thickness (px) of the invisible in-content edge margin used to grab window
/// resize, in the full-bleed layout that has no native frame.
const RESIZE_EDGE: f64 = 8.0;
/// Minimum window size enforced while edge-resizing.
const MIN_WINDOW: i32 = 160;

pub struct App {
    state: Rc<RefCell<ViewState>>,
    image: Rc<RefCell<Option<LoadedImage>>>,
    anim: Rc<RefCell<Option<Player>>>,
    playback: Rc<RefCell<Option<Playback>>>,
    settings: Rc<RefCell<Settings>>,
}

impl App {
    pub fn new() -> Self {
        Self {
            state: Rc::new(RefCell::new(ViewState::default())),
            image: Rc::new(RefCell::new(None)),
            anim: Rc::new(RefCell::new(None)),
            playback: Rc::new(RefCell::new(None)),
            settings: Rc::new(RefCell::new(Settings::load())),
        }
    }

    pub fn run(&mut self, filename: Option<String>) -> glib::ExitCode {
        let app = Application::builder()
            .application_id("io.github.gary-host-laptop.opti")
            .build();

        let state = self.state.clone();
        let image = self.image.clone();
        let anim = self.anim.clone();
        let playback = self.playback.clone();
        let settings = self.settings.clone();

        app.connect_activate(move |app| {
            let window = ApplicationWindow::builder()
                .application(app)
                .title("opti")
                .default_width(800)
                .default_height(600)
                .decorated(true)
                .build();

            // Transparent window background via CSS (the GTK4 replacement for
            // GTK3's rgba_visual + app_paintable). The window is transparent so
            // the desktop shows through the rounded corners, and content is
            // truly full-bleed: no titlebar slot, so the image fills the whole
            // window edge-to-edge up to the very top, with a single transparent
            // headerbar of regular titlebar height floated on top carrying the
            // default close button.
            let provider = CssProvider::new();
            provider.load_from_data(
                "window { \
                     background-color: rgba(0, 0, 0, 0.0); \
                     border-radius: 12px; \
                 } \
                 headerbar.opti-controls { \
                     background: transparent; \
                     border: none; \
                     box-shadow: none; \
                 } \
                 headerbar.opti-controls .title { color: transparent; }",
            );
            let display = gdk::Display::default().expect("no display");
            gtk4::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk4::STYLE_PROVIDER_PRIORITY_USER,
            );

            // Full-bleed content under a transparent control bar, Clapper style.
            let overlay = gtk4::Overlay::new();

            let drawing = DrawingArea::new();
            drawing.set_draw_func({
                let state = state.clone();
                let image = image.clone();
                move |_, cr, w, h| {
                    // Clip to the rounded window shape so the image and overlays
                    // don't paint over the transparent corner pixels.
                    render::clip_window(cr, w as f64, h as f64);
                    let st = state.borrow();
                    let img = image.borrow();
                    if let Some(img) = img.as_ref() {
                        render::draw_image(cr, &st, img, w as f64, h as f64);
                    }
                    render::draw_name_overlay(cr, &st, w as f64, h as f64);
                    render::draw_help_overlay(cr, &st, w as f64, h as f64);
                }
            });
            overlay.set_child(Some(&drawing));

            // Transparent control bar of regular titlebar height over the
            // content, with the system-default window buttons (single close on
            // GNOME).
            let controls = gtk4::HeaderBar::new();
            controls.add_css_class("opti-controls");
            controls.set_hexpand(true);
            controls.set_valign(gtk4::Align::Start);
            controls.set_show_title_buttons(true);
            overlay.add_overlay(&controls);

            window.set_child(Some(&overlay));

            // --- input handled in helper methods on this closure ---
            setup_input(&window, &drawing, &state, &image, &anim, &playback, &settings);

            // Persistent ticker: advances animation playback and pulls decoded
            // video frames into the image cell.
            {
                let drawing = drawing.clone();
                let image = image.clone();
                let anim = anim.clone();
                let playback = playback.clone();
                let state = state.clone();
                let mut last_video_seq = 0u64;
                glib::timeout_add_local(std::time::Duration::from_millis(8), move || {
                    let mut updated = false;
                    if let Some(player) = anim.borrow_mut().as_mut() {
                        if player.advance(8.0) {
                            let frame = player.frame();
                            let mut img = image.borrow_mut();
                            match img.as_mut() {
                                Some(existing)
                                    if existing.width == frame.width
                                        && existing.height == frame.height =>
                                {
                                    existing.pixels.clone_from(&frame.pixels);
                                }
                                _ => {
                                    *img = Some(frame.clone());
                                }
                            }
                            drop(img);
                            updated = true;
                        }
                    }
                    if !updated {
                        if let Some(pb) = playback.borrow().as_ref() {
                            let fr = pb.frame.lock().unwrap();
                            if fr.seq != last_video_seq {
                                last_video_seq = fr.seq;
                                let w = fr.width;
                                let h = fr.height;
                                let pixels = fr.pixels.clone();
                                drop(fr);
                                {
                                    let mut img = image.borrow_mut();
                                    match img.as_mut() {
                                        Some(existing)
                                            if existing.width == w && existing.height == h =>
                                        {
                                            existing.pixels.clone_from(&pixels);
                                            existing.kind = MediaKind::Video;
                                        }
                                        _ => {
                                            *img = Some(LoadedImage {
                                                width: w,
                                                height: h,
                                                pixels,
                                                kind: MediaKind::Video,
                                            });
                                        }
                                    }
                                }
                                {
                                    let mut st = state.borrow_mut();
                                    if st.width != w as u32 || st.height != h as u32 {
                                        st.width = w as u32;
                                        st.height = h as u32;
                                    }
                                }
                                updated = true;
                            }
                        }
                    }
                    if updated {
                        drawing.queue_draw();
                    }
                    glib::ControlFlow::Continue
                });
            }

            // Restore last window geometry, then track move/resize for saving.
            restore_window_bounds(&window, &settings.borrow());
            track_window_bounds(&window, &settings, &state);

            window.present();

            // Debug: confirm we're on the X11 backend and the raw X11 window
            // handle resolves (the XWayland path M5 relies on).
            if playback::debug_log() {
                if let Some(surface) = window.surface() {
                    let backend = surface.display().downcast_ref::<gdk4_x11::X11Display>().is_some();
                    let raw = xwin::raw_for(&surface);
                    eprintln!(
                        "opti: [x11] backend={backend} raw={} origin={:?}",
                        raw.is_some(),
                        raw.as_ref().map(|r| r.origin()),
                    );
                }
            }

            // Load a file provided on the command line, if any.
            if let Some(name) = filename.as_deref() {
                if name.starts_with("http://") || name.starts_with("https://") {
                    state.borrow_mut().context = Context::Url;
                    open_url(
                        &drawing,
                        &state,
                        &image,
                        &anim,
                        &playback,
                        &settings,
                        name.to_string(),
                    );
                } else {
                    load_path(
                        &drawing,
                        &state,
                        &image,
                        &anim,
                        &playback,
                        &settings,
                        std::path::PathBuf::from(name),
                    );
                }
            }
        });

        app.run_with_args(&[] as &[&str])
    }
}

fn setup_input(
    window: &ApplicationWindow,
    drawing: &DrawingArea,
    state: &Rc<RefCell<ViewState>>,
    image: &Rc<RefCell<Option<LoadedImage>>>,
    anim: &Rc<RefCell<Option<Player>>>,
    playback: &Rc<RefCell<Option<Playback>>>,
    settings: &Rc<RefCell<Settings>>,
) {
    let controller = gtk4::EventControllerKey::new();

    let window_handle = window.clone();
    let state_rc = state.clone();
    let image_rc = image.clone();
    let anim_rc = anim.clone();
    let playback_rc = playback.clone();
    let settings_rc = settings.clone();
    let drawing_handle = drawing.clone();
    controller.connect_key_pressed(move |_, key, _code, modifiers| {
        let mut guard = match state_rc.try_borrow_mut() {
            Ok(g) => g,
            Err(_) => return glib::Propagation::Proceed,
        };
        let st = &mut *guard;
        let control = modifiers.contains(gdk::ModifierType::CONTROL_MASK);
        let shift = modifiers.contains(gdk::ModifierType::SHIFT_MASK);
        let alt = modifiers.contains(gdk::ModifierType::ALT_MASK);
        st.ctrl_held = control;
        st.shift_held = shift;
        st.alt_held = alt;

        let name = key
            .name()
            .map(|s| s.to_string())
            .unwrap_or_default()
            .to_ascii_lowercase();

        let media_active = matches!(st.media, Some(MediaKind::Video) | Some(MediaKind::Audio));

        // Determine a navigation action (a target path) if any, else a pure
        // state flip. Navigation actions need to drop the borrow before loading.
        let mut nav_target: Option<std::path::PathBuf> = None;
        let mut acted = false;

        // Side effects that must run after the state borrow is released.
        let mut aot: Option<bool> = None; // always-on-top (EWMH)
        let mut passthrough: Option<bool> = None; // cursor pass-through
        let mut f11_toggle = false;
        let mut reload = false;
        let mut restart = false;

        match name.as_str() {
            "escape" => {
                window_handle.close();
                acted = true;
            }
            "a" => {
                st.pinned = !st.pinned;
                aot = Some(st.pinned);
                acted = true;
            }
            "p" => {
                st.passthrough = !st.passthrough;
                passthrough = Some(st.passthrough);
                acted = true;
            }
            "f11" => {
                f11_toggle = true;
                acted = true;
            }
            "return" | "enter" => {
                // Open the parent folder like opti's "enter open folder".
                nav_target = match st.fullpath.as_ref() {
                    Some(p) if p.is_file() => p.parent().map(|d| d.to_path_buf()),
                    _ => parent_directory(&st),
                };
            }
            "r" => {
                if shift {
                    restart = true;
                } else {
                    reload = true;
                }
                acted = true;
            }
            "l" => {
                st.window_locked = !st.window_locked;
                acted = true;
            }
            "n" => {
                st.show_name = !st.show_name;
                acted = true;
            }
            "?" | "/" => {
                st.show_help = !st.show_help;
                acted = true;
            }
            "f" => {
                if shift {
                    st.flip_y = !st.flip_y;
                } else {
                    st.flip_x = !st.flip_x;
                }
                acted = true;
            }
            "space" => {
                if media_active && !shift {
                    if let Some(p) = playback_rc.borrow().as_ref() {
                        p.toggle_pause();
                    }
                    acted = true;
                } else {
                    st.reset();
                    if let Some(player) = anim_rc.borrow_mut().as_mut() {
                        player.reset();
                        if let Some(frame) = image_rc.borrow_mut().as_mut() {
                            frame.pixels.clone_from(&player.frame().pixels);
                        }
                    }
                    acted = true;
                }
            }
            ">" | "." => {
                if media_active {
                    if let Some(p) = playback_rc.borrow().as_ref() {
                        if shift {
                            p.shift_seek(1);
                        } else {
                            p.step_frames(1);
                        }
                    }
                    acted = true;
                } else if let Some(player) = anim_rc.borrow_mut().as_mut() {
                    player.step(1);
                    if let Some(frame) = image_rc.borrow_mut().as_mut() {
                        frame.pixels.clone_from(&player.frame().pixels);
                    }
                    acted = true;
                }
            }
            "<" | "," => {
                if media_active {
                    if let Some(p) = playback_rc.borrow().as_ref() {
                        if shift {
                            p.shift_seek(-1);
                        } else {
                            p.step_frames(-1);
                        }
                    }
                    acted = true;
                } else if let Some(player) = anim_rc.borrow_mut().as_mut() {
                    player.step(-1);
                    if let Some(frame) = image_rc.borrow_mut().as_mut() {
                        frame.pixels.clone_from(&player.frame().pixels);
                    }
                    acted = true;
                }
            }
            "s" => {
                let mut synced = false;
                if media_active {
                    if let Some(p) = playback_rc.borrow().as_ref() {
                        p.sync_to_epoch();
                        synced = true;
                    }
                }
                if !synced {
                    // Animated images: restart playback at a fresh anchor.
                    if let Some(player) = anim_rc.borrow_mut().as_mut() {
                        player.sync_restart();
                        if let Some(frame) = image_rc.borrow_mut().as_mut() {
                            frame.pixels.clone_from(&player.frame().pixels);
                        }
                        synced = true;
                    }
                }
                if synced {
                    acted = true;
                }
            }
            "minus" => {
                let mut set = settings_rc.borrow_mut();
                if let Some(p) = playback_rc.borrow().as_ref() {
                    p.nudge_volume(-1.0, &mut set);
                } else {
                    let v = (set.volume() - VOLUME_STEP).max(0.0);
                    set.set_volume(v);
                    set.save();
                }
                acted = true;
            }
            "equal" | "plus" => {
                let mut set = settings_rc.borrow_mut();
                if let Some(p) = playback_rc.borrow().as_ref() {
                    p.nudge_volume(1.0, &mut set);
                } else {
                    let v = (set.volume() + VOLUME_STEP).min(1.0);
                    set.set_volume(v);
                    set.save();
                }
                acted = true;
            }
            "up" => {
                if alt {
                    nav_target = parent_directory(&st);
                } else if !shift {
                    st.adjust_zoom(1);
                    acted = true;
                }
            }
            "down" => {
                if !shift {
                    st.adjust_zoom(-1);
                    acted = true;
                }
            }
            "right" => {
                nav_target = if shift {
                    navigation::step_media(&mut *st, 1)
                } else {
                    navigation::step_file(&mut *st, 1)
                };
            }
            "left" => {
                nav_target = if shift {
                    navigation::step_media(&mut *st, -1)
                } else {
                    navigation::step_file(&mut *st, -1)
                };
            }
            "backspace" => {
                nav_target = parent_directory(&st);
            }
            "o" => {
                if let Some(fp) = st.fullpath.clone() {
                    reveal_in_explorer(&fp);
                    acted = true;
                }
            }
            "c" => {
                if control {
                    if let Some(display) = gdk::Display::default() {
                        let clipboard = display.clipboard();
                        copy_current(&image_rc, &clipboard, alt);
                    }
                    acted = true;
                }
            }
            "v" => {
                if control {
                    if let Some(display) = gdk::Display::default() {
                        let clipboard = display.clipboard();
                        paste_from_clipboard(
                            &drawing_handle,
                            &state_rc,
                            &image_rc,
                            &anim_rc,
                            &playback_rc,
                            &settings_rc,
                            &clipboard,
                        );
                    }
                    acted = true;
                }
            }
            _ => {
                acted = false;
            }
        }
        let _ = control;

        drop(guard);

        // Apply GTK/X11 side effects now that the state borrow is released.
        if let Some(on) = aot {
            set_always_on_top(&window_handle, on);
        }
        if let Some(_on) = passthrough {
            set_passthrough(&window_handle, &state_rc.borrow());
        }
        if f11_toggle {
            if window_handle.is_fullscreen() {
                window_handle.unfullscreen();
            } else {
                window_handle.fullscreen();
            }
        }
        if reload {
            reload_current(&drawing_handle, &state_rc, &image_rc, &anim_rc, &playback_rc, &settings_rc);
        }
        if restart {
            restart_self();
        }

        if let Some(path) = nav_target {
            load_path(
                &drawing_handle,
                &state_rc,
                &image_rc,
                &anim_rc,
                &playback_rc,
                &settings_rc,
                path,
            );
            glib::Propagation::Stop
        } else if acted {
            drawing_handle.queue_draw();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });

    drawing.add_controller(controller);

    // Pointer gestures (drag-move/pan/rotate/scale, edge-resize, scroll) and
    // the top-drag scrim hint.
    attach_pointer_input(window, drawing, state, settings);

    // Drag-and-drop: files and uri-list load the first dropped item.
    attach_drag_drop(drawing, state, image, anim, playback, settings);
}

/// Pointer gestures: drag-to-move, ctrl+drag pan, right-drag rotate,
/// alt+right-drag scale, edge-resize, and scroll (pixel-scale / zoom /
/// opacity). Also paints a translucent top-drag scrim hint.
fn attach_pointer_input(
    window: &ApplicationWindow,
    drawing: &DrawingArea,
    state: &Rc<RefCell<ViewState>>,
    settings: &Rc<RefCell<Settings>>,
) {
    let state_rc = state.clone();
    let window_rc = window.clone();
    let drawing_rc = drawing.clone();
    let settings_rc = settings.clone();
    let drawing_sc = drawing.clone();

    let legacy = gtk4::EventControllerLegacy::new();
    legacy.connect_event(move |_, event| {
        let modifiers = event.modifier_state();
        let control = modifiers.contains(gdk::ModifierType::CONTROL_MASK);
        let shift = modifiers.contains(gdk::ModifierType::SHIFT_MASK);
        let alt = modifiers.contains(gdk::ModifierType::ALT_MASK);

        {
            let mut st = state_rc.borrow_mut();
            st.ctrl_held = control;
            st.shift_held = shift;
            st.alt_held = alt;
        }

        let pos = event.position();
        let (px, py) = pos.unwrap_or((0.0, 0.0));

        // Actual window client size (not the loaded image size).
        let (win_w, win_h) = window_rc
            .surface()
            .map(|s| (s.width() as f64, s.height() as f64))
            .unwrap_or((0.0, 0.0));

        let mut st = state_rc.borrow_mut();
        let evt_type = event.event_type();

        if evt_type == gdk::EventType::MotionNotify {
            st.cur_x = px;
            st.cur_y = py;
            // Drive the active drag. Window move/resize is handled natively by
            // the WM via the (transparent) CSD titlebar and frame edges, so no
            // in-content hover/edge handling is needed here.
            if st.dragging && st.mouse_down && !st.mouse_right_down {
                pointer_drag_move(&mut st, px, py, &window_rc, &drawing_rc);
            } else if st.mouse_right_down {
                pointer_drag_right(&mut st, px, py, shift, alt);
                drawing_rc.queue_draw();
            }
            return glib::Propagation::Proceed;
        }

        if let Some(button) = event.downcast_ref::<gdk::ButtonEvent>().map(|b| b.button()) {
            match evt_type {
                gdk::EventType::ButtonPress => {
                    st.cur_x = px;
                    st.cur_y = py;
                    if button == xwin::BUTTON_LEFT {
                        st.mouse_down = true;
                        pointer_press_left(
                            &mut st,
                            px,
                            py,
                            control,
                            shift,
                            alt,
                            &window_rc,
                            win_w,
                            win_h,
                            button,
                        );
                    } else if button == xwin::BUTTON_RIGHT {
                        st.mouse_right_down = true;
                        pointer_press_right(&mut st, px, py, shift, alt);
                    }
                    drawing_rc.queue_draw();
                }
                gdk::EventType::ButtonRelease => {
                    if button == xwin::BUTTON_LEFT {
                        st.mouse_down = false;
                        st.dragging = false;
                    } else if button == xwin::BUTTON_RIGHT {
                        st.mouse_right_down = false;
                    }
                    if !st.mouse_down && !st.mouse_right_down {
                        st.drag_mode = DragMode::None;
                        save_window_bounds(&window_rc, &settings_rc.borrow());
                    }
                    drawing_rc.queue_draw();
                }
                _ => {}
            }
        }
        glib::Propagation::Proceed
    });
    drawing.add_controller(legacy);

    // Scroll: shift = opacity; ctrl/alt/default = image zoom (stage).
    let scroll = gtk4::EventControllerScroll::new(gtk4::EventControllerScrollFlags::VERTICAL);
    let state_sc = state.clone();
    scroll.connect_scroll(move |_, _, dy| {
        let (ctrl, shift, alt) = {
            let st = state_sc.borrow();
            (st.ctrl_held, st.shift_held, st.alt_held)
        };
        let mut st = state_sc.borrow_mut();
        handle_scroll(&mut st, dy, ctrl, shift, alt);
        drawing_sc.queue_draw();
        glib::Propagation::Stop
    });
    drawing.add_controller(scroll);
}

/// Begin a left-button drag, choosing edge-resize vs. pan by context.
fn pointer_press_left(
    st: &mut ViewState,
    px: f64,
    py: f64,
    control: bool,
    _shift: bool,
    _alt: bool,
    _window: &ApplicationWindow,
    win_w: f64,
    win_h: f64,
    _button: u32,
) {
    st.drag_start_x = px;
    st.drag_start_y = py;
    st.pan_start_x = st.pan_x;
    st.pan_start_y = st.pan_y;

    if control {
        // Ctrl+drag always pans the image, locked or not.
        st.drag_mode = DragMode::Pan;
    } else {
        // Full-bleed window has no native frame: a left-drag starting on a thin
        // edge/corner margin resizes the window in-content instead.
        st.resize_east = win_w - px <= RESIZE_EDGE && px > 0.0;
        st.resize_south = win_h - py <= RESIZE_EDGE && py > 0.0;
        if st.resize_east || st.resize_south {
            st.resize_start_w = win_w;
            st.resize_start_h = win_h;
            st.drag_mode = DragMode::Resize;
        } else {
            st.resize_east = false;
            st.resize_south = false;
            st.drag_mode = DragMode::None;
        }
    }
    st.dragging = st.drag_mode != DragMode::None;
}

fn pointer_press_right(st: &mut ViewState, px: f64, py: f64, shift: bool, alt: bool) {
    st.drag_start_x = px;
    st.drag_start_y = py;
    st.start_angle = st.angle;
    st.zoom_stage_start = st.zoom_stage;
    st.drag_mode = if alt { DragMode::Scale } else { DragMode::Rotate };
    st.dragging = true;
    let _ = shift;
}

/// Advance a pan drag, or a window edge-resize drag.
fn pointer_drag_move(
    st: &mut ViewState,
    px: f64,
    py: f64,
    window: &ApplicationWindow,
    drawing: &DrawingArea,
) {
    let dx = px - st.drag_start_x;
    let dy = py - st.drag_start_y;
    if st.drag_mode == DragMode::Pan {
        st.pan_x = st.pan_start_x + dx;
        st.pan_y = st.pan_start_y + dy;
        drawing.queue_draw();
    } else if st.drag_mode == DragMode::Resize {
        let mut w = st.resize_start_w;
        let mut h = st.resize_start_h;
        if st.resize_east {
            w += dx;
        }
        if st.resize_south {
            h += dy;
        }
        window.set_default_size(w.max(MIN_WINDOW) as i32, h.max(MIN_WINDOW) as i32);
    }
}

/// Drive a right-button drag (rotate, or alt drag-scale).
fn pointer_drag_right(st: &mut ViewState, px: f64, py: f64, shift: bool, alt: bool) {
    let dx = px - st.drag_start_x;
    let dy = py - st.drag_start_y;
    if alt {
        // Drag-scale: vertical distance changes the zoom stage.
        st.zoom_stage = st.zoom_stage_start + (dy / 100.0).round() as i32;
        st.update_zoom();
    } else {
        // Rotate toward the angle of the pointer relative to the grab point.
        let a = dy.atan2(dx).to_degrees();
        let mut ang = st.start_angle + a;
        if shift {
            ang = (ang / 15.0).round() * 15.0;
        }
        st.angle = ang;
    }
}

/// Fetch the X11 raw handles for this window's surface, if available.
fn current_raw(window: &ApplicationWindow) -> Option<xwin::Raw> {
    let surface = window.surface()?;
    xwin::raw_for(&surface)
}

/// Toggle the EWMH always-on-top hint for the window.
fn set_always_on_top(window: &ApplicationWindow, on: bool) {
    if let Some(raw) = current_raw(window) {
        raw.set_always_on_top(on);
    }
}

/// Toggle cursor pass-through: an empty input region ignores all pointer
/// input (keys still work, so the app can't get stuck).
fn set_passthrough(window: &ApplicationWindow, state: &ViewState) {
    reapply_input_region(window, state);
    if playback::debug_log() {
        eprintln!("opti: [input] passthrough = {}", state.passthrough);
    }
}

/// Reload the currently loaded file through the normal load path.
fn reload_current(
    drawing: &DrawingArea,
    state: &Rc<RefCell<ViewState>>,
    image: &Rc<RefCell<Option<LoadedImage>>>,
    anim: &Rc<RefCell<Option<Player>>>,
    playback: &Rc<RefCell<Option<Playback>>>,
    settings: &Rc<RefCell<Settings>>,
) {
    let path = state.borrow().fullpath.clone();
    if let Some(p) = path {
        load_path(drawing, state, image, anim, playback, settings, p);
    }
}

/// Restart the process with the same arguments (opti's shift+r).
fn restart_self() {
    use std::process::{Command, Stdio};
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(_) => return,
    };
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Ok(_child) = Command::new(&exe).args(&args).stdin(Stdio::null()).spawn() {
        std::process::exit(0);
    }
}

/// Apply a scroll event: shift = opacity; ctrl/alt/default = zoom stage.
fn handle_scroll(st: &mut ViewState, dy: f64, control: bool, shift: bool, alt: bool) {
    let dir = if dy < 0.0 { 1 } else { -1 };
    if shift {
        st.opacity = (st.opacity + dir as f64 * 0.05).clamp(0.0, 1.0);
    } else if control || alt {
        st.adjust_zoom(dir);
    } else {
        st.adjust_zoom(dir);
    }
}

/// Accept file/URI drops onto the drawing area and route the first item in.
fn attach_drag_drop(
    drawing: &DrawingArea,
    state: &Rc<RefCell<ViewState>>,
    image: &Rc<RefCell<Option<LoadedImage>>>,
    anim: &Rc<RefCell<Option<Player>>>,
    playback: &Rc<RefCell<Option<Playback>>>,
    settings: &Rc<RefCell<Settings>>,
) {
    let target = gtk4::DropTarget::new(glib::Type::INVALID, gdk::DragAction::COPY);
    let (state, image, anim, playback, settings) = (
        state.clone(),
        image.clone(),
        anim.clone(),
        playback.clone(),
        settings.clone(),
    );
    let drawing_cb = drawing.clone();
    target.connect_drop(move |_, value, _, _| {
        if let Ok(file_list) = value.get::<gdk::FileList>() {
            let files = file_list.files();
            if let Some(first) = files.first() {
                let path = first.path();
                if let Some(p) = path {
                    load_path(&drawing_cb, &state, &image, &anim, &playback, &settings, p);
                    return true;
                }
            }
        }
        if let Ok(text) = value.get::<String>() {
            open_text(&drawing_cb, &state, &image, &anim, &playback, &settings, &text);
            return true;
        }
        if playback::debug_log() {
            eprintln!("opti: [drop] unsupported payload");
        }
        false
    });
    drawing.add_controller(target);
}

/// Copy the currently displayed image to the clipboard: a PNG bitmap, or a
/// `data:` URI text payload when `data_uri` is set.
fn copy_current(
    image: &Rc<RefCell<Option<LoadedImage>>>,
    clipboard: &gdk::Clipboard,
    data_uri: bool,
) {
    let img = image.borrow();
    let img = match img.as_ref() {
        Some(img) if img.width > 0 && img.height > 0 => img,
        _ => return,
    };
    let png = match loader::encode_png_rgba(&img.pixels, img.width as u32, img.height as u32) {
        Some(p) => p,
        None => return,
    };
    if data_uri {
        let text = network::encode_data_uri("image/png", &png);
        let provider = gdk::ContentProvider::for_bytes(
            "text/plain;charset=utf-8",
            &glib::Bytes::from(text.as_bytes()),
        );
        let _ = clipboard.set_content(Some(&provider));
    } else {
        let provider = gdk::ContentProvider::for_bytes("image/png", &glib::Bytes::from(png.as_slice()));
        let _ = clipboard.set_content(Some(&provider));
    }
}

/// Paste a bitmap or text from the clipboard (mirrors opti's readImage ->
/// readText fallback).
#[allow(clippy::too_many_arguments)]
fn paste_from_clipboard(
    drawing: &DrawingArea,
    state: &Rc<RefCell<ViewState>>,
    image: &Rc<RefCell<Option<LoadedImage>>>,
    anim: &Rc<RefCell<Option<Player>>>,
    playback: &Rc<RefCell<Option<Playback>>>,
    settings: &Rc<RefCell<Settings>>,
    clipboard: &gdk::Clipboard,
) {
    let (drawing, state, image, anim, playback, settings) = (
        drawing.clone(),
        state.clone(),
        image.clone(),
        anim.clone(),
        playback.clone(),
        settings.clone(),
    );
    let clipboard_handle = clipboard.clone();
    clipboard
        .read_texture_async(None::<&gio::Cancellable>, move |res| match res {
            Ok(Some(texture)) => {
                let w = texture.width() as usize;
                let h = texture.height() as usize;
                let mut buf = vec![0u8; w * h * 4];
                texture.download(&mut buf, w * 4);
                {
                    let mut st = state.borrow_mut();
                    st.context = Context::Url;
                    st.width = w as u32;
                    st.height = h as u32;
                    st.fullpath = None;
                    st.filepath = None;
                    st.filename = None;
                    st.media = Some(MediaKind::Image(image::ImageFormat::Png));
                }
                *image.borrow_mut() = Some(LoadedImage {
                    width: w as i32,
                    height: h as i32,
                    pixels: buf,
                    kind: MediaKind::Image(image::ImageFormat::Png),
                });
                *anim.borrow_mut() = None;
                if let Some(p) = playback.borrow_mut().take() {
                    p.stop();
                }
                drawing.queue_draw();
            }
            _ => {
                clipboard_handle.read_text_async(None::<&gio::Cancellable>, move |res| {
                    if let Ok(Some(text)) = res {
                        open_text(
                            &drawing, &state, &image, &anim, &playback, &settings,
                            &text,
                        );
                    }
                });
            }
        });
}

/// Restore the last window size from data.json. (GTK4 removed window-position
/// APIs, so only size is persisted.)
fn restore_window_bounds(window: &ApplicationWindow, settings: &Settings) {
    if let (Some(w), Some(h)) = (settings.get_i32("bounds_w"), settings.get_i32("bounds_h")) {
        if w >= 120 && h >= 120 {
            window.set_default_size(w, h);
        }
    }
}

/// Save window size to data.json, debounced like opti's setData.
fn save_window_bounds(window: &ApplicationWindow, settings: &Settings) {
    if let Some(surface) = window.surface() {
        let w = surface.width();
        let h = surface.height();
        if w <= 0 || h <= 0 {
            return;
        }
        let mut s = settings.clone();
        s.set_i32("bounds_w", w);
        s.set_i32("bounds_h", h);
        s.save();
    }
}

/// Track user resizes (via the window surface's width/height notify) and
/// persist the size after a short debounce. Also re-applies the input region
/// (passthrough state) so a resized window never leaves stale click-through
/// geometry behind.
///
/// Debounce uses a shared dirty flag + a single repeating timer rather than
/// cancelable one-shot sources: cancelling a one-shot `SourceId` after it has
/// already dispatched panics ("Failed to remove source"), which raw X11
/// resizes (several width/height notifies in quick succession) can trigger.
fn track_window_bounds(
    window: &ApplicationWindow,
    settings: &Rc<RefCell<Settings>>,
    state: &Rc<RefCell<ViewState>>,
) {
    let settings = settings.clone();
    let state = state.clone();
    let wired = Rc::new(RefCell::new(false));
    window.connect_map(move |window| {
        if *wired.borrow() {
            return;
        }
        *wired.borrow_mut() = true;
        let Some(surface) = window.surface() else {
            return;
        };
        let dirty_w = Rc::new(RefCell::new(false));
        let dirty_h = Rc::new(RefCell::new(false));
        {
            let dw = dirty_w.clone();
            surface.connect_width_notify(move |_| {
                *dw.borrow_mut() = true;
            });
        }
        {
            let dh = dirty_h.clone();
            surface.connect_height_notify(move |_| {
                *dh.borrow_mut() = true;
            });
        }

        let winrc = window.clone();
        let (settings, state) = (settings.clone(), state.clone());
        glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
            let changed = *dirty_w.borrow() || *dirty_h.borrow();
            if changed {
                *dirty_w.borrow_mut() = false;
                *dirty_h.borrow_mut() = false;
                save_window_bounds(&winrc, &settings.borrow());
                reapply_input_region(&winrc, &state.borrow());
            }
            glib::ControlFlow::Continue
        });
    });
}

/// Re-apply the pointer input region, matching the current passthrough state
/// (and the current surface size). Call after any resize so clicks aren't
/// dropped outside a stale region.
fn reapply_input_region(window: &ApplicationWindow, state: &ViewState) {
    let Some(surface) = window.surface() else {
        return;
    };
    if state.passthrough {
        let region = cairo::Region::create_rectangle(&cairo::RectangleInt::new(0, 0, 0, 0));
        surface.set_input_region(&region);
    } else {
        // A fresh (default) surface already accepts input everywhere; leaving
        // the region untouched restores full interaction after a resize.
        surface.set_input_region(&{
            let w = surface.width();
            let h = surface.height();
            cairo::Region::create_rectangle(&cairo::RectangleInt::new(0, 0, w, h))
        });
    }
}

/// Load a path (file or directory): populate nav state and render the image.
fn load_path(
    drawing: &DrawingArea,
    state: &Rc<RefCell<ViewState>>,
    image: &Rc<RefCell<Option<LoadedImage>>>,
    anim: &Rc<RefCell<Option<Player>>>,
    playback: &Rc<RefCell<Option<Playback>>>,
    settings: &Rc<RefCell<Settings>>,
    path: std::path::PathBuf,
) {
    // Stop any running media playback before switching content.
    if let Some(p) = playback.borrow_mut().take() {
        p.stop();
    }

    let is_dir = std::fs::metadata(&path).map(|m| m.is_dir()).unwrap_or(false);
    if is_dir {
        if let Some((list, idx)) = navigation::load_directory(&path) {
            let mut st = state.borrow_mut();
            st.local_files = list;
            st.file_index = idx;
            st.context = Context::Directory;
            st.fullpath = Some(path.clone());
            st.filepath = Some(path.clone());
            st.filename = None;
            st.media = None;
            *image.borrow_mut() = None;
            *anim.borrow_mut() = None;
        }
        drawing.queue_draw();
        return;
    }

    // Populate the directory listing around this file.
    if let Some((list, idx)) = navigation::load_directory(&path) {
        let mut st = state.borrow_mut();
        st.local_files = list;
        st.file_index = idx;
        st.context = Context::File;
    }

    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_default();
    match std::fs::read(&path) {
        Ok(bytes) => ingest_bytes(
            drawing, state, image, anim, playback, settings, bytes, &ext, Some(path),
        ),
        Err(_) => {}
    }
    drawing.queue_draw();
}

/// Route raw media bytes into the viewer (same logic used for files, pasted
/// data, dropped URLs, and downloaded content).
#[allow(clippy::too_many_arguments)]
fn ingest_bytes(
    drawing: &DrawingArea,
    state: &Rc<RefCell<ViewState>>,
    image: &Rc<RefCell<Option<LoadedImage>>>,
    anim: &Rc<RefCell<Option<Player>>>,
    playback: &Rc<RefCell<Option<Playback>>>,
    settings: &Rc<RefCell<Settings>>,
    bytes: Vec<u8>,
    ext: &str,
    path: Option<std::path::PathBuf>,
) {
    // Stop any running media playback before switching content.
    if let Some(p) = playback.borrow_mut().take() {
        p.stop();
    }

    let kind = loader::detect_media(&bytes, ext);
    if kind.is_animated() {
        if let Some(animdata) = anim::decode(&bytes, ext) {
            let player = Player::start(animdata);
            let frame = player.frame().clone();
            let mut st = state.borrow_mut();
            st.width = frame.width as u32;
            st.height = frame.height as u32;
            st.fullpath = path.clone();
            st.filepath = path.as_ref().and_then(|p| p.parent().map(|p| p.to_path_buf()));
            st.filename = path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|f| f.to_string_lossy().to_string());
            st.media = Some(kind);
            *image.borrow_mut() = Some(frame);
            *anim.borrow_mut() = Some(player);
        } else {
            *anim.borrow_mut() = None;
        }
    } else if matches!(kind, MediaKind::Video | MediaKind::Audio) {
        // Video/audio: hand the file to GStreamer; decoded video frames flow
        // into the image cell via the main-thread ticker. A real path is
        // required (GStreamer needs a seekable source), so synthesize a temp
        // file when the bytes came without one.
        let media_path = match &path {
            Some(p) if p.exists() => p.clone(),
            _ => {
                let suffix = match ext {
                    "" => "bin",
                    e => e,
                };
                match network::write_temp_file(&bytes, suffix) {
                    Ok(p) => p,
                    Err(e) => {
                        if playback::debug_log() {
                            eprintln!("opti: [playback] temp write failed: {e}");
                        }
                        return;
                    }
                }
            }
        };
        {
            let mut st = state.borrow_mut();
            st.width = 0;
            st.height = 0;
            st.fullpath = Some(media_path.clone());
            st.filepath = media_path.parent().map(|p| p.to_path_buf());
            st.filename = media_path
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .or_else(|| path.as_ref().and_then(|p| p.file_name()).map(|f| f.to_string_lossy().to_string()));
            st.media = Some(kind.clone());
            *image.borrow_mut() = None;
            *anim.borrow_mut() = None;
        }
        let uri = gio::File::for_path(&media_path).uri();
        let volume = settings.borrow().volume();
        if playback::debug_log() {
            eprintln!("opti: [playback] routing {:?} -> {:?}", media_path, kind);
        }
        if let Some(pb) = Playback::new(&uri, volume) {
            *playback.borrow_mut() = Some(pb);
        } else if playback::debug_log() {
            eprintln!("opti: [playback] FAILED to start {}", uri);
        }
    } else if let Some(img) = loader::load(&bytes, ext) {
        if playback::debug_log() {
            eprintln!(
                "opti: [load] image {}x{} from {}",
                img.width,
                img.height,
                path.as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "(clipboard/url)".to_string())
            );
        }
        let mut st = state.borrow_mut();
        st.width = img.width as u32;
        st.height = img.height as u32;
        st.fullpath = path.clone();
        st.filepath = path.as_ref().and_then(|p| p.parent().map(|p| p.to_path_buf()));
        st.filename = path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|f| f.to_string_lossy().to_string());
        st.media = Some(img.kind.clone());
        *image.borrow_mut() = Some(img);
        *anim.borrow_mut() = None;
    }
    drawing.queue_draw();
}

/// Return the parent directory path of the currently loaded file/folder, if any.
fn parent_directory(state: &ViewState) -> Option<std::path::PathBuf> {
    state
        .filepath
        .as_ref()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
}

/// Reveal the parent folder of `path` in the file manager.
fn reveal_in_explorer(path: &std::path::Path) {
    if let Some(parent) = path.parent() {
        let uri = gio::File::for_path(parent).uri();
        let _ = gio::AppInfo::launch_default_for_uri(uri.as_str(), None::<&gio::AppLaunchContext>);
    }
}

/// Return the file extension (no dot) for a URL, or "" if none.
fn url_suffix(url: &str) -> String {
    let path = url.split('?').next().unwrap_or(url);
    path.rsplit('/')
        .next()
        .and_then(|seg| {
            seg.rsplit_once('.')
                .map(|(_, ext)| ext.to_ascii_lowercase())
        })
        .unwrap_or_default()
}

/// Open a clipboard/dropped text payload: file path, `file://` URI, `data:`
/// URI, or http(s) URL.
#[allow(clippy::too_many_arguments)]
fn open_text(
    drawing: &DrawingArea,
    state: &Rc<RefCell<ViewState>>,
    image: &Rc<RefCell<Option<LoadedImage>>>,
    anim: &Rc<RefCell<Option<Player>>>,
    playback: &Rc<RefCell<Option<Playback>>>,
    settings: &Rc<RefCell<Settings>>,
    text: &str,
) {
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    if let Some(bytes) = network::decode_data_uri(text) {
        // Data URI: sniff the actual payload; hand to ingest_bytes.
        state.borrow_mut().context = Context::Url;
        let ext = data_uri_ext(text);
        ingest_bytes(drawing, state, image, anim, playback, settings, bytes, &ext, None);
        return;
    }
    let scheme = || {
        text.split_once(':')
            .map(|(s, _)| s.to_ascii_lowercase())
    };
    if text.starts_with("http://") || text.starts_with("https://") {
        state.borrow_mut().context = Context::Url;
        open_url(
            drawing, state, image, anim, playback, settings,
            text.to_string(),
        );
    } else if matches!(scheme(), Some(s) if s == "file") {
        if let Ok(p) = url_encoded_path_of(text) {
            state.borrow_mut().context = Context::File;
            load_path(drawing, state, image, anim, playback, settings, p);
        }
    } else {
        let p = std::path::PathBuf::from(text);
        if p.exists() {
            state.borrow_mut().context = Context::File;
            load_path(drawing, state, image, anim, playback, settings, p);
        }
    }
}

fn data_uri_ext(uri: &str) -> String {
    let head = uri.split(',').next().unwrap_or("");
    match head {
        h if h.contains("gif") => "gif".to_string(),
        h if h.contains("png") => "png".to_string(),
        h if h.contains("jpeg") || h.contains("jpg") => "jpg".to_string(),
        h if h.contains("webp") => "webp".to_string(),
        h if h.contains("svg") => "svg".to_string(),
        h if h.contains("bmp") => "bmp".to_string(),
        h if h.contains("avif") => "avif".to_string(),
        h if h.contains("video/mp4") => "mp4".to_string(),
        h if h.contains("audio/ogg") => "ogg".to_string(),
        h if h.contains("audio/mpeg") => "mp3".to_string(),
        h if h.contains("audio/wav") => "wav".to_string(),
        h if h.contains("audio/webm") => "webm".to_string(),
        h if h.contains("video/webm") => "webm".to_string(),
        _ => "bin".to_string(),
    }
}

/// Convert a `file:///path` URI into a PathBuf.
fn url_encoded_path_of(uri: &str) -> Result<std::path::PathBuf, ()> {
    let p = uri.split_once("file://").map(|(_, r)| r).ok_or(())?;
    // Decode %20-style escapes (fractions of URL encoding; good enough here).
    let decoded = percent_decode(p);
    Ok(std::path::PathBuf::from(decoded))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Fetch an http(s) URL and route its content into the viewer.
#[allow(clippy::too_many_arguments)]
fn open_url(
    drawing: &DrawingArea,
    state: &Rc<RefCell<ViewState>>,
    image: &Rc<RefCell<Option<LoadedImage>>>,
    anim: &Rc<RefCell<Option<Player>>>,
    playback: &Rc<RefCell<Option<Playback>>>,
    settings: &Rc<RefCell<Settings>>,
    url: String,
) {
    let label = url.split('?').next().unwrap_or(&url).to_string();
    let suffix = url_suffix(&url);
    let (drawing, state, image, anim, playback, settings) = (
        drawing.clone(),
        state.clone(),
        image.clone(),
        anim.clone(),
        playback.clone(),
        settings.clone(),
    );
    glib::spawn_future_local(async move {
        let bytes = match network::fetch_url(&url) {
            Ok(b) => {
                if playback::debug_log() {
                    eprintln!("opti: [network] fetched {} ({} bytes)", url, b.len());
                }
                b
            }
            Err(e) => {
                if playback::debug_log() {
                    eprintln!("opti: [network] {e}");
                }
                return;
            }
        };
        ingest_bytes(
            &drawing, &state, &image, &anim, &playback, &settings,
            bytes, &suffix, Some(label.into()),
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ViewState;

    #[test]
    fn handle_scroll_clamps_opacity() {
        let mut st = ViewState::default();
        // Scroll up increases opacity; ensure no overflow past 1.0.
        for _ in 0..1000 {
            handle_scroll(&mut st, -10.0, false, true, false);
        }
        assert!(st.opacity <= 1.0);
        for _ in 0..1000 {
            handle_scroll(&mut st, 10.0, false, true, false);
        }
        assert!(st.opacity >= 0.0);
    }

    #[test]
    fn handle_scroll_adjusts_zoom_stage() {
        let mut st = ViewState::default();
        let before = st.zoom_stage;
        handle_scroll(&mut st, -10.0, true, false, false); // ctrl scroll up => zoom in
        assert!(st.zoom_stage > before);
    }

    #[test]
    fn right_drag_rotate_respects_snap_quantum() {
        let mut st = ViewState::default();
        st.angle = 0.0;
        st.drag_start_x = 0.0;
        st.drag_start_y = 0.0;
        // Pointer moved 45 degrees down-right from origin.
        pointer_drag_right(&mut st, 10.0, 10.0, true, false);
        assert_eq!(st.angle, 45.0);
        // Without snap it stays the raw atan2 angle.
        st.angle = 0.0;
        pointer_drag_right(&mut st, 10.0, 5.0, false, false);
        assert!((st.angle - 26.565).abs() < 0.1 || (st.angle - 26.565 + 360.0).abs() < 0.1);
    }

    #[test]
    fn right_drag_scale_changes_zoom_stage() {
        let mut st = ViewState::default();
        st.zoom_stage_start = 0;
        st.drag_start_x = 0.0;
        st.drag_start_y = 0.0;
        // Dragging down (positive dy) zooms in, matching legacy opti.
        pointer_drag_right(&mut st, 0.0, 200.0, false, true);
        assert_eq!(st.zoom_stage, 2); // up by 2 stages == zoom 3x
        assert!((st.zoom - 3.0).abs() < 1e-6);
        // Dragging up (negative dy) zooms out.
        pointer_drag_right(&mut st, 0.0, -200.0, false, true);
        assert_eq!(st.zoom_stage, -2); // == 1/3x
    }
}

