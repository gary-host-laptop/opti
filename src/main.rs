use clap::Parser;

mod anim;
mod app;
mod loader;
mod media;
mod navigation;
mod network;
mod playback;
mod render;
mod settings;
mod state;
mod xwin;

/// A simple, transparent media viewer.
#[derive(Parser, Debug)]
#[command(name = "opti", version, about)]
struct Args {
    /// File(s) to open.
    #[arg(value_name = "FILE")]
    files: Vec<String>,
}

fn main() -> glib::ExitCode {
    // Run on the X11 backend (XWayland) so we can drive raw X11 window
    // geometry and EWMH state (always-on-top, free move/resize). Must be set
    // before any GDK/GLib display initialization.
    std::env::set_var("GDK_BACKEND", "x11");

    let args = Args::parse();
    let filename = args.files.clone().into_iter().next();

    let mut app = app::App::new();
    app.run(filename)
}
