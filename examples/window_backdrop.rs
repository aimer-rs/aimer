//! A window frosted like a macOS sidebar: the desktop behind it shows through,
//! blurred, wherever the content is not opaque.
//!
//! The window is transparent, so nothing here paints a full-window background:
//! the text sits straight on the blur.
//!
//! ```text
//! cargo run -p aimer --example window_backdrop
//! ```

use aimer::{AimerApp, Text, WindowAttr, WindowBackdrop, WindowMaterial};

fn main() {
    AimerApp::new()
        .window(WindowAttr::new().title("Window backdrop").inner_size(520, 360))
        .child(
            WindowBackdrop::new()
                .material(WindowMaterial::Glass)
                .child(Text::new("This window blurs what is behind it")),
        )
        .run();
}
