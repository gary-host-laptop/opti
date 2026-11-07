//! Raw X11 window control via the XWayland surface that GDK runs on.
//!
//! GDK removes GTK3's always-on-top API, so opti targets the X11 backend
//! explicitly (`GDK_BACKEND=x11`) and drives the EWMH _NET_WM_STATE_ABOVE
//! ClientMessage on the XWayland surface. Window move/resize is left to the
//! window manager's native CSD decorations (transparent titlebar + frame
//! edges), so this module only needs the window id for the always-on-top hint
//! and absolute-origin queries.

use std::os::raw::{c_char, c_int, c_long, c_ulong};

use gtk4::gdk;
use gtk4::prelude::*;

use gdk4_x11::x11::xlib as xl;
use gdk4_x11::{X11Display, X11Surface};

const CLIENT_MESSAGE: c_int = 33;

/// The number of the first mouse button (left); used for drag intent.
pub const BUTTON_LEFT: u32 = 1;
pub const BUTTON_RIGHT: u32 = 3;

/// Raw handles for the X11 display and our top-level window.
pub struct Raw {
    display: *mut xl::Display,
    root: xl::Window,
    win: xl::Window,
}

/// Retrieve the raw X11 handles for `surface`'s display. Returns None when the
/// process didn't actually end up on the X11 backend (shouldn't happen given
/// GDK_BACKEND=x11, but be defensive).
pub fn raw_for(surface: &gdk::Surface) -> Option<Raw> {
    let display = surface.display();
    let xdisplay = display.downcast_ref::<X11Display>()?;
    let xsurface = surface.downcast_ref::<X11Surface>()?;
    Some(Raw {
        display: unsafe { xdisplay.xdisplay() },
        root: xdisplay.xrootwindow(),
        win: xsurface.xid(),
    })
}

impl Raw {
    /// Absolute top-left of the client area, translated to the root window.
    pub fn origin(&self) -> (i32, i32) {
        let root = self.root;
        let mut x: c_int = 0;
        let mut y: c_int = 0;
        let mut child: c_ulong = 0;
        unsafe {
            xl::XTranslateCoordinates(
                self.display, self.win, root, 0, 0, &mut x, &mut y, &mut child,
            );
        }
        (x, y)
    }

    /// Toggle the EWMH `_NET_WM_STATE_ABOVE` hint via a ClientMessage to the
    /// root window (the only reliable way on an already-mapped toplevel).
    pub fn set_always_on_top(&self, on: bool) {
        let ewmh = b"_NET_WM_STATE\0";
        let above = b"_NET_WM_STATE_ABOVE\0";
        // EWMH strings may legitimately contain (POSIX) non-UTF8 bytes; send
        // the raw byte pointer and null terminator as char*.
        let ewmh_atom = unsafe { self.xatom_bytes(ewmh) };
        let above_atom = unsafe { self.xatom_bytes(above) };
        if ewmh_atom == 0 || above_atom == 0 {
            return;
        }
        let action = if on { 1 } else { 0 }; // _NET_WM_STATE_ADD / REMOVE

        let mut data = xl::ClientMessageData::new();
        data.set_long(0, action);
        data.set_long(1, above_atom as c_long);
        data.set_long(2, 0);
        data.set_long(3, 1); // source indication: normal application
        data.set_long(4, 0);

        let msg = xl::XClientMessageEvent {
            type_: CLIENT_MESSAGE,
            serial: 0,
            send_event: 1,
            display: self.display,
            window: self.win,
            message_type: ewmh_atom,
            format: 32,
            data,
        };

        let mut ev: xl::XEvent = unsafe { std::mem::zeroed() };
        ev.client_message = msg;
        let mask = xl::SubstructureNotifyMask | xl::SubstructureRedirectMask;
        unsafe {
            xl::XSendEvent(self.display, self.root, 0, mask, &mut ev);
            xl::XFlush(self.display);
        }
    }

    /// Resolve an atom by name. Returns 0 if the display can't intern it.
    unsafe fn xatom_bytes(&self, name: &[u8]) -> xl::Atom {
        let cstr = match std::ffi::CString::new(name) {
            Ok(c) => c,
            Err(_) => return 0,
        };
        xl::XInternAtom(self.display, cstr.as_ptr() as *const c_char, 0) as xl::Atom
    }
}
