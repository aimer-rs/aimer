use crate::{AimerApp, RenderMode};

#[test]
fn native_apps_default_to_threaded_and_allow_inline_configuration() {
    #[cfg(not(target_arch = "wasm32"))]
    assert_eq!(
        AimerApp::new().configured_render_mode(),
        RenderMode::Threaded
    );
    let app = AimerApp::new().render_mode(RenderMode::Inline);
    assert_eq!(app.configured_render_mode(), RenderMode::Inline);
}
