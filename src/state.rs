use crate::media::MediaKind;

#[derive(Clone, PartialEq)]
pub enum Context {
    File,
    Directory,
    Url,
    None,
}

/// The active pointer-drag intent.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DragMode {
    None,
    /// Ctrl+left drag: pan the image within the window.
    Pan,
    /// Right-drag without alt: rotate the image.
    Rotate,
    /// Alt + right-drag: drag-scale (vertical distance -> zoom).
    Scale,
    /// Left-drag from a window edge/corner: resize the window (full-bleed
    /// layout has no native frame, so resize is handled in-content).
    Resize,
}

#[derive(Clone)]
pub struct ViewState {
    pub media: Option<MediaKind>,
    pub filepath: Option<std::path::PathBuf>,
    pub filename: Option<String>,
    pub fullpath: Option<std::path::PathBuf>,
    pub local_files: Vec<std::path::PathBuf>,
    pub file_index: usize,
    pub context: Context,

    pub width: u32,
    pub height: u32,

    pub zoom: f64,
    pub zoom_stage: i32,
    pub pan_x: f64,
    pub pan_y: f64,
    pub pan_start_x: f64,
    pub pan_start_y: f64,
    pub angle: f64,
    pub start_angle: f64,
    pub opacity: f64,

    pub flip_x: bool,
    pub flip_y: bool,

    pub pinned: bool,
    pub passthrough: bool,
    pub window_locked: bool,
    pub show_name: bool,
    pub show_help: bool,

    // transient pointer-drag state
    pub mouse_down: bool,
    pub mouse_right_down: bool,
    pub dragging: bool,
    pub drag_start_x: f64,
    pub drag_start_y: f64,
    pub cur_x: f64,
    pub cur_y: f64,

    // M5 pointer/gesture intent
    pub drag_mode: DragMode,
    /// Zoom stage captured at drag-start (for drag-scale).
    pub zoom_stage_start: i32,
    /// Window resize drag state: which edges are active and the size at onset.
    pub resize_east: bool,
    pub resize_south: bool,
    pub resize_start_w: f64,
    pub resize_start_h: f64,
    /// Live modifier state, mirrored from pointer/key events for use in
    /// gestures that don't report modifiers themselves.
    pub ctrl_held: bool,
    pub shift_held: bool,
    pub alt_held: bool,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            media: None,
            filepath: None,
            filename: None,
            fullpath: None,
            local_files: Vec::new(),
            file_index: 0,
            context: Context::None,
            width: 0,
            height: 0,
            zoom: 1.0,
            zoom_stage: 0,
            pan_x: 0.0,
            pan_y: 0.0,
            pan_start_x: 0.0,
            pan_start_y: 0.0,
            angle: 0.0,
            start_angle: 0.0,
            opacity: 1.0,
            flip_x: false,
            flip_y: false,
            pinned: false,
            passthrough: false,
            window_locked: false,
            show_name: false,
            show_help: false,
            mouse_down: false,
            mouse_right_down: false,
            dragging: false,
            drag_start_x: 0.0,
            drag_start_y: 0.0,
            cur_x: 0.0,
            cur_y: 0.0,
            drag_mode: DragMode::None,
            zoom_stage_start: 0,
            resize_east: false,
            resize_south: false,
            resize_start_w: 0.0,
            resize_start_h: 0.0,
            ctrl_held: false,
            shift_held: false,
            alt_held: false,
        }
    }
}

impl ViewState {
    pub fn reset(&mut self) {
        self.pan_x = 0.0;
        self.pan_y = 0.0;
        self.pan_start_x = 0.0;
        self.pan_start_y = 0.0;
        self.angle = 0.0;
        self.start_angle = 0.0;
        self.zoom = 1.0;
        self.zoom_stage = 0;
        self.flip_x = false;
        self.flip_y = false;
        self.opacity = 1.0;
    }

    /// Increment the zoom-stage counter and recompute `zoom` from it, matching
    /// opti's behavior (stages >= 0 are 1.25x steps, < 0 are fractional).
    pub fn adjust_zoom(&mut self, dir: i32) {
        if dir < 0 {
            self.zoom_stage -= 1;
        } else if dir > 0 {
            self.zoom_stage += 1;
        }
        self.update_zoom();
    }

    pub fn update_zoom(&mut self) {
        if self.zoom_stage >= 0 {
            self.zoom = (self.zoom_stage + 1) as f64;
        } else {
            self.zoom = 1.0 / (self.zoom_stage.unsigned_abs() as f64 + 1.0);
        }
    }
}
