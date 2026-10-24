extern crate aimer_widget;

#[cfg(all(feature = "wasm-hot-reload", target_arch = "wasm32"))]
compile_error!("Aimer's wasm-hot-reload host feature is available only on native targets");
#[cfg(all(feature = "wasm-hot-reload", not(debug_assertions)))]
compile_error!("Aimer's wasm-hot-reload host feature is available only in debug builds");

/// Interpreted application runtime exposed only to hot-reload hosts.
#[cfg(feature = "wasm-hot-reload")]
pub use aimer_anteros as wasm_runtime;
/// Authenticated development reload protocol exposed only to hot-reload hosts.
#[cfg(feature = "wasm-hot-reload")]
pub use aimer_reload_protocol as reload_protocol;
/// App-side development listener exposed only to hot-reload hosts.
#[cfg(feature = "wasm-hot-reload")]
pub use aimer_reload_server as reload_server;

#[cfg(target_os = "android")]
mod android_storage;
#[cfg(target_os = "android")]
pub use android_storage::register_android_app;
#[cfg(all(unix, not(any(target_os = "android", target_os = "ios"))))]
pub(crate) use aimer_platform::shutdown_signals;
pub(crate) use aimer_platform::{ffi_utils, link_inbox};
mod first_frame;
mod window_locals;
#[cfg(any(target_os = "ios", test))]
mod ios_input;
pub use aimer_render::RenderMode;
#[cfg(test)]
mod render_mode_tests;
#[cfg(debug_assertions)]
mod debug_overlay;

#[macro_use]
pub mod aimer_app;
pub mod handler;
/// Concrete Aimer Widget IR schemas and disconnected native materialization.
#[cfg(feature = "wasm-hot-reload")]
pub use aimer_hot_reload as hot_reload;
#[cfg(feature = "wasm-hot-reload")]
pub use aimer_hot_reload::initialize_hot_reload_host;
/// Keeps generated application entry points uniform when hot reload is absent.
///
/// Native AOT builds call this no-op without compiling any listener, protocol,
/// or interpreter code into the application.
#[cfg(not(feature = "wasm-hot-reload"))]
#[inline]
pub const fn initialize_hot_reload_host() -> bool {
    true
}
pub use aimer_app::{AimerApp, HeadlessAimerApp, HeadlessOptions};
pub use aimer_cupid::AntiAlias;
pub use first_frame::{FIRST_FRAME_RENDERED_EVENT, set_first_frame_rendered_callback};
#[cfg(target_os = "ios")]
mod ios_screen {
    pub use crate::ffi_utils::ios_screen::attach_window_to_active_scene;
}

mod adapter_detail;
pub use aimer_frame_stats as frame_stats;
/// The native application menu, and the shortcuts macOS routes through it.
pub use aimer_platform::menu;
/// Entering the application's async runtime while Venus polls a task. Native
/// only: the browser has one task queue and no runtime to be inside of.
#[cfg(not(target_arch = "wasm32"))]
pub use aimer_platform::poll_context;
#[cfg(target_arch = "wasm32")]
pub(crate) use aimer_platform::browser_poll_context;
/// Off-thread rasterization. Native only: the browser has no thread the WebGPU
/// objects could move to, so the web backend presents inline.
#[cfg(not(target_arch = "wasm32"))]
pub use aimer_render::raster;
pub use aimer_render::render_ctx;
pub(crate) use aimer_platform::{system_appearance, system_safe_area};
pub use aimer_window::window_attr;
pub use aimer_window::{ExitPolicy, NewWindow, Window, WindowAttr, WindowId};
/// Opening, controlling and closing windows.
pub use aimer_window as windows;
#[cfg(not(any(target_os = "ios", target_os = "android", target_arch = "wasm32")))]
mod desktop_host;

pub use winit;

#[cfg(test)]
mod tests {
    #[cfg(feature = "wasm-hot-reload")]
    mod candidate_preparation {
        use std::cell::RefCell;
        use std::fs;
        use std::net::TcpStream;
        use std::path::PathBuf;
        use std::process::Command;
        use std::rc::Rc;
        use std::sync::{OnceLock, mpsc};
        use std::thread;
        use std::time::{Duration, Instant};

        use crate::AimerApp;
        use crate::hot_reload::{
            EVENT_BUTTON_PRESS, LiveReloadConfig, LiveReloadHost, ReloadCandidateLimits,
            ReloadCandidatePreparationError, ReloadCandidatePreparer,
            materialize_aimer_widget_tree, reload_command_bridge,
        };
        use crate::reload_protocol::{
            ModuleMetadata, ProtocolLimits, ReloadResult, ReloadStage as ProtocolReloadStage,
            SessionCredentials, send_reload_command,
        };
        use crate::reload_server::{ListenerSecurity, ReloadCommandListener};
        use aimer_anteros::{
            CallbackBinding, CallbackBindingSnapshot, CallbackEvent, CapabilityCompletionToken,
            CapabilityDescriptor, CapabilityGeneration, CapabilityLimits, CapabilityProvider,
            CapabilityRegistry, CapabilityResult, CapabilityStagingClass, Generation, GenerationId,
            GenerationLimits, GuestInstance, ModelLimits, PROPERTY_TEXT_CONTENT, PropertyValue,
            ReloadSnapshot, ReloadStage, Runtime, RuntimeConfig, RuntimeErrorKind, StableId128,
            StateBundleView, StateTransferCoordinator, Version, WIDGET_BUTTON, WIDGET_TEXT,
            WidgetDocument, WidgetNode, WidgetProperty,
        };
        use aimer_widget::base::{BuildContext, WindowHandle};
        use aimer_widget::{AnyElement, Widget};
        use winit::dpi::PhysicalPosition;
        use winit::event::{DeviceId, ElementState, MouseButton, WindowEvent};

        const MODEL_LIMITS: ModelLimits =
            ModelLimits::new(4_096, 32, 128, 128).max_widget_depth(16);
        const CALLBACK_ID: StableId128 = StableId128::from_bytes([0x22; 16]);
        const ACTIVE_GENERATION: GenerationId = GenerationId::new(6);
        const CANDIDATE_GENERATION: GenerationId = GenerationId::new(7);

        #[allow(dead_code)]
        mod capability_guest {
            include!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../host/anteros/tests/support/guest_module.rs"
            ));
        }

        #[allow(dead_code)]
        #[derive(aimer_macro::PortableWidget)]
        #[portable_widget(id = "aimer_quiver.tests.Phase23SchemaOnly", schema_only)]
        struct Phase23SchemaOnly;

        impl Widget for Phase23SchemaOnly {
            fn to_element(self, ctx: &BuildContext) -> AnyElement {
                aimer_text::Text::new("schema-only").to_element(ctx)
            }
        }

        #[test]
        fn authenticated_initial_module_commits_on_the_live_host_safe_point() {
            let credentials = SessionCredentials::from_parts([0x11; 16], [0xA5; 32]);
            let protocol_limits = ProtocolLimits::new(16 * 1_024 * 1_024, Duration::from_secs(10))
                .max_chunk_bytes(64 * 1_024);
            let (wake_sender, wake_receiver) = mpsc::sync_channel(1);
            let mut host = LiveReloadHost::bind(
                "127.0.0.1:0",
                credentials.clone(),
                LiveReloadConfig::new(
                    runtime_config(),
                    protocol_limits,
                    ListenerSecurity::new(Duration::from_secs(60), 4, Duration::from_secs(1)),
                    ReloadCandidateLimits::new(MODEL_LIMITS, GenerationLimits::new(4), 8),
                )
                .state_transfer(state_transfer())
                .max_queued_events(8),
                aimer_venus::LocalScheduler::new(),
                move || {
                    let _ = wake_sender.try_send(());
                },
            )
            .unwrap();
            let address = host.local_addr();
            let client_module = stateful_guest_module().to_vec();
            let client = thread::spawn(move || {
                let mut stream = TcpStream::connect(address).unwrap();
                send_reload_command(
                    &mut stream,
                    &credentials,
                    protocol_limits,
                    40,
                    ModuleMetadata::new([1; 16], [2; 16], 1, 0, [3; 32]),
                    &client_module,
                )
                .unwrap()
            });

            wake_receiver.recv_timeout(Duration::from_secs(10)).unwrap();
            let commit = host
                .process_safe_point(&context())
                .unwrap()
                .expect("the authenticated initial module must commit");

            assert_eq!(commit.generation_id(), GenerationId::new(1));
            assert_eq!(host.active_generation(), Some(GenerationId::new(1)));
            assert_eq!(host.active_root().unwrap().debug_name(), "Column");
            assert!(matches!(
                client.join().unwrap(),
                ReloadResult::Committed {
                    active_generation: 1,
                    reset_state_entries: 0,
                    cleanup_warnings: 0,
                }
            ));
            assert!(matches!(
                wake_receiver.try_recv(),
                Err(mpsc::TryRecvError::Empty)
            ));
        }

        #[test]
        fn host_owned_typed_capability_completion_uses_generation_identity_and_rejects_stale_delivery()
         {
            let completion = Rc::new(RefCell::new(None));
            let mut capabilities = CapabilityRegistry::new(1);
            capabilities
                .register_with_staging(
                    AsyncCapabilityProvider {
                        completion: Rc::clone(&completion),
                    },
                    CapabilityStagingClass::PureQuery,
                )
                .unwrap();
            let credentials = SessionCredentials::from_parts([0x14; 16], [0xA8; 32]);
            let protocol_limits = ProtocolLimits::new(16 * 1_024 * 1_024, Duration::from_secs(10))
                .max_chunk_bytes(64 * 1_024);
            let (wake_sender, wake_receiver) = mpsc::sync_channel(1);
            let mut host = LiveReloadHost::bind(
                "127.0.0.1:0",
                credentials.clone(),
                LiveReloadConfig::new(
                    runtime_config(),
                    protocol_limits,
                    ListenerSecurity::new(Duration::from_secs(60), 4, Duration::from_secs(1)),
                    ReloadCandidateLimits::new(MODEL_LIMITS, GenerationLimits::new(4), 8),
                )
                .capabilities(capabilities)
                .state_transfer(state_transfer())
                .max_queued_events(8),
                aimer_venus::LocalScheduler::new(),
                move || {
                    let _ = wake_sender.try_send(());
                },
            )
            .unwrap();
            let address = host.local_addr();
            let first = send_fixture_module(
                address,
                &credentials,
                protocol_limits,
                46,
                &capability_guest::capability_async_guest(),
            );
            wake_receiver.recv_timeout(Duration::from_secs(10)).unwrap();
            assert_eq!(
                host.process_safe_point(&context())
                    .unwrap()
                    .unwrap()
                    .generation_id(),
                GenerationId::new(1)
            );
            assert!(matches!(
                first.join().unwrap(),
                ReloadResult::Committed {
                    active_generation: 1,
                    ..
                }
            ));

            let completion = completion
                .borrow_mut()
                .take()
                .expect("typed provider must receive the active capability token");
            let task_id = host.register_active_async_task(CALLBACK_ID).unwrap();
            let typed_result = completion
                .complete(TypedCapabilityResult { value: 0xCAFE })
                .unwrap();
            let payload = typed_result.value.to_le_bytes();
            let event = completion
                .encode_async_completion(CALLBACK_ID, task_id, 1, &payload, MODEL_LIMITS)
                .unwrap();
            host.dispatch_async_event(&event, &context()).unwrap();
            let state = host
                .export_active_state()
                .unwrap()
                .expect("the active guest must export state after capability completion");
            let state_view = StateBundleView::decode(&state, MODEL_LIMITS).unwrap();
            assert_eq!(
                state_view.entry(0).unwrap().payload(),
                [1],
                "the generated guest must apply the typed completion to bounded state"
            );
            assert!(matches!(
                host.dispatch_async_event(&event, &context()),
                Err(crate::hot_reload::LiveReloadError::Callback(message))
                    if message.contains("async task")
            ));

            let second = send_fixture_module(
                address,
                &credentials,
                protocol_limits,
                47,
                &capability_guest::capability_async_guest(),
            );
            wake_receiver.recv_timeout(Duration::from_secs(10)).unwrap();
            assert_eq!(
                host.process_safe_point(&context())
                    .unwrap()
                    .unwrap()
                    .generation_id(),
                GenerationId::new(2)
            );
            assert!(matches!(
                second.join().unwrap(),
                ReloadResult::Committed {
                    active_generation: 2,
                    ..
                }
            ));

            assert_eq!(
                completion.complete(TypedCapabilityResult { value: 0xBEEF }),
                Err(aimer_anteros::CapabilityError::RetiredGeneration)
            );
            assert!(matches!(
                host.dispatch_async_event(&event, &context()),
                Err(crate::hot_reload::LiveReloadError::Callback(message))
                    if message.contains("generation")
            ));
        }

        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        struct TypedCapabilityResult {
            value: u16,
        }

        struct AsyncCapabilityProvider {
            completion: Rc<RefCell<Option<CapabilityCompletionToken>>>,
        }

        impl CapabilityProvider for AsyncCapabilityProvider {
            fn descriptor(&self) -> CapabilityDescriptor {
                CapabilityDescriptor::new(
                    StableId128::from_bytes([0x20; 16]),
                    1,
                    [0x30; 32],
                    CapabilityLimits::new(0, 512),
                )
            }

            fn invoke(
                &self,
                generation: CapabilityGeneration,
                method_id: u32,
                request: &[u8],
                response_limit: u32,
            ) -> CapabilityResult<Vec<u8>> {
                assert_eq!(method_id, 0);
                assert!(request.is_empty());
                assert_eq!(response_limit, 512);
                self.completion.replace(Some(generation.completion_token()));
                let text_properties = [WidgetProperty::new(
                    PROPERTY_TEXT_CONTENT,
                    PropertyValue::StringRef(0),
                )];
                let child = [1_u32];
                let callbacks = [CallbackBinding::new_async(
                    EVENT_BUTTON_PRESS,
                    Version::new(1, 0),
                    Version::new(1, 0),
                    StableId128::from_bytes([0x22; 16]),
                )];
                let nodes = [
                    WidgetNode::new(WIDGET_BUTTON, Version::new(1, 0))
                        .callbacks(&callbacks)
                        .children(&child),
                    WidgetNode::new(WIDGET_TEXT, Version::new(1, 0)).properties(&text_properties),
                ];
                WidgetDocument::new(
                    generation.id().get(),
                    1,
                    0,
                    &nodes,
                    &["typed capability"],
                    &[],
                )
                .encode(MODEL_LIMITS)
                .map_err(|_| aimer_anteros::CapabilityError::LimitExceeded)
            }
        }

        #[test]
        fn dropping_live_host_cancels_a_queued_command_without_blocking_exit() {
            let credentials = SessionCredentials::from_parts([0x13; 16], [0xA7; 32]);
            let protocol_limits = ProtocolLimits::new(16 * 1_024 * 1_024, Duration::from_secs(10))
                .max_chunk_bytes(64 * 1_024);
            let (wake_sender, wake_receiver) = mpsc::sync_channel(1);
            let host = LiveReloadHost::bind(
                "127.0.0.1:0",
                credentials.clone(),
                LiveReloadConfig::new(
                    runtime_config(),
                    protocol_limits,
                    ListenerSecurity::new(Duration::from_secs(60), 4, Duration::from_secs(1)),
                    ReloadCandidateLimits::new(MODEL_LIMITS, GenerationLimits::new(4), 8),
                )
                .state_transfer(state_transfer())
                .max_queued_events(8),
                aimer_venus::LocalScheduler::new(),
                move || {
                    let _ = wake_sender.try_send(());
                },
            )
            .unwrap();
            let address = host.local_addr();
            let client_module = stateful_guest_module().to_vec();
            let client = thread::spawn(move || {
                let mut stream = TcpStream::connect(address).unwrap();
                send_reload_command(
                    &mut stream,
                    &credentials,
                    protocol_limits,
                    45,
                    ModuleMetadata::new([1; 16], [2; 16], 1, 0, [6; 32]),
                    &client_module,
                )
                .unwrap()
            });

            wake_receiver.recv_timeout(Duration::from_secs(10)).unwrap();
            drop(host);

            assert!(matches!(
                client.join().unwrap(),
                ReloadResult::Rejected {
                    stage: crate::reload_protocol::ReloadStage::Cancellation,
                    active_generation: 0,
                    ..
                }
            ));
        }

        #[test]
        fn configured_app_installs_an_authenticated_module_in_its_shared_frame_path() {
            let credentials = SessionCredentials::from_parts([0x12; 16], [0xA6; 32]);
            let protocol_limits = ProtocolLimits::new(16 * 1_024 * 1_024, Duration::from_secs(10))
                .max_chunk_bytes(64 * 1_024);
            let mut app = AimerApp::new()
                .hot_reload(
                    "127.0.0.1:0".parse().unwrap(),
                    credentials.clone(),
                    LiveReloadConfig::new(
                        runtime_config(),
                        protocol_limits,
                        ListenerSecurity::new(Duration::from_secs(60), 4, Duration::from_secs(1)),
                        ReloadCandidateLimits::new(MODEL_LIMITS, GenerationLimits::new(4), 8),
                    )
                    .state_transfer(state_transfer())
                    .max_queued_events(8),
                )
                .child(aimer_text::Text::new("native placeholder"))
                .run_headless();
            app.render_frame();
            let address = app.live_reload_addr().unwrap();
            let client_module = stateful_guest_module().to_vec();
            let first_credentials = credentials.clone();
            let client = thread::spawn(move || {
                let mut stream = TcpStream::connect(address).unwrap();
                send_reload_command(
                    &mut stream,
                    &first_credentials,
                    protocol_limits,
                    42,
                    ModuleMetadata::new([1; 16], [2; 16], 1, 0, [3; 32]),
                    &client_module,
                )
                .unwrap()
            });

            let deadline = Instant::now() + Duration::from_secs(10);
            while !app.take_redraw_request() {
                assert!(
                    Instant::now() < deadline,
                    "live listener did not wake the app"
                );
                thread::yield_now();
            }
            app.render_frame();

            assert_eq!(app.live_reload_generation(), Some(GenerationId::new(1)));
            assert_eq!(app.active_root_name(), Some("Column"));
            assert!(matches!(
                client.join().unwrap(),
                ReloadResult::Committed {
                    active_generation: 1,
                    reset_state_entries: 0,
                    cleanup_warnings: 0,
                }
            ));
            let (button_start, button_end) = app
                .active_element_bounds("Container")
                .expect("button container must be laid out");
            let button_width = button_end.x - button_start.x;
            let button_height = button_end.y - button_start.y;
            assert!(
                button_width > 0.0 && button_height > 0.0,
                "{button_start:?}..{button_end:?}"
            );
            let device_id = DeviceId::dummy();
            app.send_window_event(WindowEvent::CursorMoved {
                device_id,
                position: PhysicalPosition::new(
                    f64::from((button_start.x + button_end.x) / 2.0),
                    f64::from((button_start.y + button_end.y) / 2.0),
                ),
            });
            app.send_window_event(WindowEvent::MouseInput {
                device_id,
                state: ElementState::Pressed,
                button: MouseButton::Left,
            });
            app.render_frame();
            app.send_window_event(WindowEvent::MouseInput {
                device_id,
                state: ElementState::Released,
                button: MouseButton::Left,
            });
            app.render_frame();
            let state = app.live_reload_state().unwrap().unwrap();
            let state = StateBundleView::decode(&state, MODEL_LIMITS).unwrap();
            assert_eq!(state.entry(0).unwrap().payload(), [1]);
            assert_eq!(app.live_reload_generation(), Some(GenerationId::new(1)));

            let client_module = stateful_guest_module().to_vec();
            let second_credentials = credentials.clone();
            let client = thread::spawn(move || {
                let mut stream = TcpStream::connect(address).unwrap();
                send_reload_command(
                    &mut stream,
                    &second_credentials,
                    protocol_limits,
                    43,
                    ModuleMetadata::new([1; 16], [2; 16], 1, 0, [4; 32]),
                    &client_module,
                )
                .unwrap()
            });
            wait_for_redraw(&app);
            app.render_frame();
            assert!(matches!(
                client.join().unwrap(),
                ReloadResult::Committed {
                    active_generation: 2,
                    reset_state_entries: 0,
                    cleanup_warnings: 0,
                }
            ));
            let state = app.live_reload_state().unwrap().unwrap();
            let state = StateBundleView::decode(&state, MODEL_LIMITS).unwrap();
            assert_eq!(state.entry(0).unwrap().payload(), [1]);
            assert_eq!(app.live_reload_generation(), Some(GenerationId::new(2)));

            let malformed_credentials = credentials.clone();
            let client = thread::spawn(move || {
                let mut stream = TcpStream::connect(address).unwrap();
                send_reload_command(
                    &mut stream,
                    &malformed_credentials,
                    protocol_limits,
                    44,
                    ModuleMetadata::new([1; 16], [2; 16], 1, 0, [5; 32]),
                    b"not a WebAssembly module",
                )
                .unwrap()
            });
            wait_for_redraw(&app);
            app.render_frame();
            assert!(matches!(
                client.join().unwrap(),
                ReloadResult::Rejected {
                    stage: crate::reload_protocol::ReloadStage::Instantiate,
                    active_generation: 2,
                    ..
                }
            ));
            let state = app.live_reload_state().unwrap().unwrap();
            let state = StateBundleView::decode(&state, MODEL_LIMITS).unwrap();
            assert_eq!(state.entry(0).unwrap().payload(), [1]);
            assert_eq!(app.live_reload_generation(), Some(GenerationId::new(2)));
            let diagnostic = app
                .live_reload_diagnostic()
                .expect("a rejected candidate must leave a host diagnostic");
            assert!(
                diagnostic.contains("candidate instantiation failed"),
                "{diagnostic}"
            );

            let recovery_module = stateful_guest_module().to_vec();
            let recovery_credentials = credentials.clone();
            let client = thread::spawn(move || {
                let mut stream = TcpStream::connect(address).unwrap();
                send_reload_command(
                    &mut stream,
                    &recovery_credentials,
                    protocol_limits,
                    45,
                    ModuleMetadata::new([1; 16], [2; 16], 1, 0, [7; 32]),
                    &recovery_module,
                )
                .unwrap()
            });
            wait_for_redraw(&app);
            app.render_frame();
            assert!(matches!(
                client.join().unwrap(),
                ReloadResult::Committed {
                    active_generation: 3,
                    reset_state_entries: 0,
                    cleanup_warnings: 0,
                }
            ));
            assert!(
                app.live_reload_diagnostic().is_none(),
                "a committed candidate must clear the old diagnostic"
            );
        }

        #[test]
        fn first_guest_rejection_keeps_native_root_and_exposes_overlay_diagnostic() {
            let credentials = SessionCredentials::from_parts([0x14; 16], [0xA8; 32]);
            let protocol_limits = ProtocolLimits::new(16 * 1_024 * 1_024, Duration::from_secs(10))
                .max_chunk_bytes(64 * 1_024);
            let mut app = AimerApp::new()
                .hot_reload(
                    "127.0.0.1:0".parse().unwrap(),
                    credentials.clone(),
                    LiveReloadConfig::new(
                        runtime_config(),
                        protocol_limits,
                        ListenerSecurity::new(Duration::from_secs(60), 4, Duration::from_secs(1)),
                        ReloadCandidateLimits::new(MODEL_LIMITS, GenerationLimits::new(4), 8),
                    )
                    .state_transfer(state_transfer())
                    .max_queued_events(8),
                )
                .child(aimer_text::Text::new("native placeholder"))
                .run_headless();
            app.render_frame();

            let address = app.live_reload_addr().unwrap();
            let rejection_credentials = credentials.clone();
            let client = thread::spawn(move || {
                let mut stream = TcpStream::connect(address).unwrap();
                send_reload_command(
                    &mut stream,
                    &rejection_credentials,
                    protocol_limits,
                    46,
                    ModuleMetadata::new([1; 16], [2; 16], 1, 0, [8; 32]),
                    b"not a WebAssembly module",
                )
                .unwrap()
            });

            wait_for_redraw(&app);
            app.render_frame();

            assert!(matches!(
                client.join().unwrap(),
                ReloadResult::Rejected {
                    stage: crate::reload_protocol::ReloadStage::Instantiate,
                    active_generation: 0,
                    ..
                }
            ));
            assert_eq!(app.live_reload_generation(), None);
            assert!(
                app.active_contains_element("RawTextWidget"),
                "the native fallback tree must remain visible"
            );
            let diagnostic = app
                .live_reload_diagnostic()
                .expect("a first-generation rejection must leave a host diagnostic");
            assert!(
                diagnostic.contains("candidate instantiation failed"),
                "{diagnostic}"
            );
        }

        #[test]
        fn production_abi_rejects_reflection_contract_failures_without_replacing_active_state() {
            let credentials = SessionCredentials::from_parts([0x15; 16], [0xA9; 32]);
            let protocol_limits = ProtocolLimits::new(16 * 1_024 * 1_024, Duration::from_secs(10))
                .max_chunk_bytes(64 * 1_024);
            let mut app = AimerApp::new()
                .hot_reload(
                    "127.0.0.1:0".parse().unwrap(),
                    credentials.clone(),
                    LiveReloadConfig::new(
                        runtime_config(),
                        protocol_limits,
                        ListenerSecurity::new(Duration::from_secs(60), 4, Duration::from_secs(1)),
                        ReloadCandidateLimits::new(MODEL_LIMITS, GenerationLimits::new(4), 8),
                    )
                    .state_transfer(state_transfer())
                    .widget_ir_diagnostics(true)
                    .max_queued_events(8),
                )
                .child(aimer_text::Text::new("native placeholder"))
                .run_headless();
            app.render_frame();

            let address = app.live_reload_addr().unwrap();
            let initial = send_fixture_module(
                address,
                &credentials,
                protocol_limits,
                50,
                stateful_guest_module(),
            );
            wait_for_redraw(&app);
            app.render_frame();
            assert_eq!(
                initial.join().unwrap(),
                ReloadResult::Committed {
                    active_generation: 1,
                    reset_state_entries: 0,
                    cleanup_warnings: 0,
                }
            );
            assert_eq!(app.live_reload_generation(), Some(GenerationId::new(1)));

            let (button_start, button_end) = app
                .active_element_bounds("Container")
                .expect("fixture button container must be laid out");
            let button_width = button_end.x - button_start.x;
            let button_height = button_end.y - button_start.y;
            assert!(
                button_width > 0.0 && button_height > 0.0,
                "{button_start:?}..{button_end:?}"
            );
            let device_id = DeviceId::dummy();
            app.send_window_event(WindowEvent::CursorMoved {
                device_id,
                position: PhysicalPosition::new(
                    f64::from((button_start.x + button_end.x) / 2.0),
                    f64::from((button_start.y + button_end.y) / 2.0),
                ),
            });
            app.send_window_event(WindowEvent::MouseInput {
                device_id,
                state: ElementState::Pressed,
                button: MouseButton::Left,
            });
            app.render_frame();
            app.send_window_event(WindowEvent::MouseInput {
                device_id,
                state: ElementState::Released,
                button: MouseButton::Left,
            });
            app.render_frame();
            assert_eq!(active_counter_from_app(&mut app), 1);

            let cases = [
                (
                    "phase23-incompatible-codec",
                    ProtocolReloadStage::Materialization,
                    "outside its Rust type range",
                ),
                (
                    "phase23-unknown-required-property",
                    ProtocolReloadStage::Materialization,
                    "unsupported required property",
                ),
                ("phase23-oversized-blob", ProtocolReloadStage::Build, "blob"),
                (
                    "phase23-missing-materializer",
                    ProtocolReloadStage::Materialization,
                    "no native materializer",
                ),
            ];

            for (index, (feature, stage, diagnostic_fragment)) in cases.into_iter().enumerate() {
                let request_id = 51 + index as u64;
                let client = send_fixture_module(
                    address,
                    &credentials,
                    protocol_limits,
                    request_id,
                    &stateful_guest_module_with_feature(feature),
                );
                wait_for_redraw(&app);
                app.render_frame();

                let result = client.join().unwrap();
                match result {
                    ReloadResult::Rejected {
                        stage: actual_stage,
                        active_generation,
                        diagnostic,
                        ..
                    } => {
                        assert_eq!(actual_stage, stage, "{feature}: {diagnostic}");
                        assert_eq!(active_generation, 1, "{feature}: {diagnostic}");
                        assert!(
                            diagnostic.contains(diagnostic_fragment),
                            "{feature}: {diagnostic}"
                        );
                    }
                    other => panic!("{feature} unexpectedly returned {other:?}"),
                }
                assert_eq!(app.live_reload_generation(), Some(GenerationId::new(1)));
                assert_eq!(active_counter_from_app(&mut app), 1);
                assert!(app.active_root_name().is_some());
            }
        }

        fn wait_for_redraw<W: aimer_widget::Widget + 'static>(app: &crate::HeadlessAimerApp<W>) {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !app.take_redraw_request() {
                assert!(
                    Instant::now() < deadline,
                    "live listener did not wake the app"
                );
                thread::yield_now();
            }
        }

        #[test]
        fn authenticated_module_bytes_prepare_a_stateful_disconnected_snapshot() {
            let credentials = SessionCredentials::from_parts([0x11; 16], [0xA5; 32]);
            let protocol_limits = ProtocolLimits::new(16 * 1_024 * 1_024, Duration::from_secs(10))
                .max_chunk_bytes(64 * 1_024);
            let (sink, inbox) = reload_command_bridge(1, ACTIVE_GENERATION.get());
            let listener = ReloadCommandListener::bind_secure(
                "127.0.0.1:0",
                credentials.clone(),
                protocol_limits,
                ListenerSecurity::new(Duration::from_secs(60), 4, Duration::from_secs(1)),
                sink,
            )
            .unwrap();
            let address = listener.local_addr().unwrap();
            let server = thread::spawn(move || listener.accept_once().unwrap());
            let client_module = stateful_guest_module().to_vec();
            let client = thread::spawn(move || {
                let mut stream = TcpStream::connect(address).unwrap();
                send_reload_command(
                    &mut stream,
                    &credentials,
                    protocol_limits,
                    41,
                    ModuleMetadata::new([1; 16], [2; 16], 1, 0, [3; 32]),
                    &client_module,
                )
                .unwrap()
            });
            let pending = inbox.recv_timeout(Duration::from_secs(10)).unwrap();
            assert_eq!(pending.command().request_id(), 41);
            let module = pending.command().module();
            let active_runtime = runtime();
            let capabilities = CapabilityRegistry::new(0);
            let mut active = active_snapshot(&active_runtime, &capabilities, module);
            let candidate_runtime = runtime();
            let state_transfer = StateTransferCoordinator::new()
                .model_limits(MODEL_LIMITS)
                .migration_fuel(10_000_000);
            let preparer = ReloadCandidatePreparer::new(
                &candidate_runtime,
                &capabilities,
                &state_transfer,
                aimer_venus::LocalScheduler::new(),
                ReloadCandidateLimits::new(MODEL_LIMITS, GenerationLimits::new(4), 8),
            );

            let prepared = preparer
                .prepare(
                    module,
                    CANDIDATE_GENERATION,
                    &mut active,
                    &context(),
                    |_| {},
                )
                .unwrap();

            assert_eq!(prepared.state_transfer_report().preserved_entries(), 1);
            assert!(
                prepared
                    .state_transfer_report()
                    .reset_state_ids()
                    .is_empty()
            );
            assert_eq!(
                active
                    .generation_mut()
                    .guest_mut()
                    .export_state(MODEL_LIMITS)
                    .unwrap()
                    .view()
                    .entry(0)
                    .unwrap()
                    .payload(),
                [1]
            );

            let mut snapshot = prepared.into_snapshot();
            assert_eq!(snapshot.generation_id(), CANDIDATE_GENERATION);
            assert_eq!(snapshot.root().debug_name(), "Column");
            assert_eq!(
                snapshot
                    .generation_mut()
                    .guest_mut()
                    .export_state(MODEL_LIMITS)
                    .unwrap()
                    .view()
                    .entry(0)
                    .unwrap()
                    .payload(),
                [1]
            );
            let event = callback_event(CANDIDATE_GENERATION, 2);
            snapshot
                .generation_mut()
                .validate_event(&event, MODEL_LIMITS)
                .unwrap();
            let cancelled = ReloadResult::Cancelled {
                active_generation: ACTIVE_GENERATION.get(),
            };
            pending.complete(cancelled.clone()).unwrap();
            assert_eq!(client.join().unwrap(), cancelled);
            assert_eq!(server.join().unwrap(), cancelled);
        }

        #[test]
        fn malformed_module_is_rejected_before_active_state_is_touched() {
            let module = stateful_guest_module();
            let active_runtime = runtime();
            let capabilities = CapabilityRegistry::new(0);
            let mut active = active_snapshot(&active_runtime, &capabilities, module);
            let candidate_runtime = runtime();
            let state_transfer = state_transfer();
            let preparer = preparer(&candidate_runtime, &capabilities, &state_transfer, 8);

            let error = match preparer.prepare(
                b"not a WebAssembly module",
                CANDIDATE_GENERATION,
                &mut active,
                &context(),
                |_| {},
            ) {
                Err(error) => error,
                Ok(_) => panic!("malformed module unexpectedly prepared a candidate"),
            };

            assert_eq!(error.stage(), ReloadStage::Instantiate);
            assert!(matches!(
                error,
                ReloadCandidatePreparationError::Instantiate(ref error)
                    if error.kind() == RuntimeErrorKind::Module
            ));
            assert_eq!(active_counter(&mut active), 1);
        }

        #[test]
        fn non_advancing_generation_is_rejected_during_preflight() {
            let module = stateful_guest_module();
            let active_runtime = runtime();
            let capabilities = CapabilityRegistry::new(0);
            let mut active = active_snapshot(&active_runtime, &capabilities, module);
            let candidate_runtime = runtime();
            let state_transfer = state_transfer();
            let preparer = preparer(&candidate_runtime, &capabilities, &state_transfer, 8);

            let error = match preparer.prepare(
                module,
                ACTIVE_GENERATION,
                &mut active,
                &context(),
                |_| {},
            ) {
                Err(error) => error,
                Ok(_) => panic!("non-advancing generation unexpectedly prepared a candidate"),
            };

            assert_eq!(error.stage(), ReloadStage::Preflight);
            assert!(matches!(
                error,
                ReloadCandidatePreparationError::GenerationNotNewer {
                    active: ACTIVE_GENERATION,
                    candidate: ACTIVE_GENERATION,
                }
            ));
            assert_eq!(active_counter(&mut active), 1);
        }

        #[test]
        fn callback_limit_rejection_discards_only_the_candidate() {
            let module = stateful_guest_module();
            let active_runtime = runtime();
            let capabilities = CapabilityRegistry::new(0);
            let mut active = active_snapshot(&active_runtime, &capabilities, module);
            let candidate_runtime = runtime();
            let state_transfer = state_transfer();
            let preparer = preparer(&candidate_runtime, &capabilities, &state_transfer, 0);

            let error = match preparer.prepare(
                module,
                CANDIDATE_GENERATION,
                &mut active,
                &context(),
                |_| {},
            ) {
                Err(error) => error,
                Ok(_) => panic!("callback limit unexpectedly admitted the candidate"),
            };

            assert_eq!(error.stage(), ReloadStage::Validate);
            assert!(matches!(
                error,
                ReloadCandidatePreparationError::Callbacks(_)
            ));
            assert_eq!(active_counter(&mut active), 1);
            active
                .generation_mut()
                .validate_event(&callback_event(ACTIVE_GENERATION, 2), MODEL_LIMITS)
                .unwrap();
        }

        #[test]
        fn repeated_preparation_uses_each_host_assigned_generation() {
            let module = stateful_guest_module();
            let bootstrap_runtime = runtime();
            let capabilities = CapabilityRegistry::new(0);
            let mut active = active_snapshot(&bootstrap_runtime, &capabilities, module);
            let first_runtime = runtime();
            let state_transfer = state_transfer();
            let first_preparer = preparer(&first_runtime, &capabilities, &state_transfer, 8);
            let first = first_preparer
                .prepare(
                    module,
                    CANDIDATE_GENERATION,
                    &mut active,
                    &context(),
                    |_| {},
                )
                .unwrap();
            let mut active = first.into_snapshot();
            active.generation_mut().guest_mut().activate();
            let second_generation = GenerationId::new(8);
            let second_runtime = runtime();
            let second_preparer = preparer(&second_runtime, &capabilities, &state_transfer, 8);

            let second = second_preparer
                .prepare(module, second_generation, &mut active, &context(), |_| {})
                .unwrap();

            assert_eq!(second.into_snapshot().generation_id(), second_generation);
        }

        fn active_snapshot(
            runtime: &Runtime,
            capabilities: &CapabilityRegistry,
            module: &[u8],
        ) -> ReloadSnapshot<GuestInstance, AnyElement> {
            let mut guest = runtime
                .instantiate_with_capabilities(
                    module,
                    capabilities,
                    MODEL_LIMITS,
                    ACTIVE_GENERATION,
                )
                .unwrap();
            guest.activate();
            guest
                .dispatch_event(&callback_event(ACTIVE_GENERATION, 1), MODEL_LIMITS)
                .unwrap();
            let image = guest.build(MODEL_LIMITS).unwrap();
            let callbacks = CallbackBindingSnapshot::from_document(&image.view(), 8).unwrap();
            let root =
                materialize_aimer_widget_tree(image.as_bytes(), MODEL_LIMITS, &context(), |_| {})
                    .unwrap();
            let generation = Generation::with_guest(
                ACTIVE_GENERATION,
                callbacks,
                aimer_venus::LocalScheduler::new(),
                GenerationLimits::new(4),
                guest,
            );
            ReloadSnapshot::new(generation, root)
        }

        fn preparer<'a>(
            runtime: &'a Runtime,
            capabilities: &'a CapabilityRegistry,
            state_transfer: &'a StateTransferCoordinator,
            max_callbacks: u32,
        ) -> ReloadCandidatePreparer<'a> {
            ReloadCandidatePreparer::new(
                runtime,
                capabilities,
                state_transfer,
                aimer_venus::LocalScheduler::new(),
                ReloadCandidateLimits::new(MODEL_LIMITS, GenerationLimits::new(4), max_callbacks),
            )
        }

        fn state_transfer() -> StateTransferCoordinator {
            StateTransferCoordinator::new()
                .model_limits(MODEL_LIMITS)
                .migration_fuel(10_000_000)
        }

        fn active_counter(active: &mut ReloadSnapshot<GuestInstance, AnyElement>) -> u8 {
            active
                .generation_mut()
                .guest_mut()
                .export_state(MODEL_LIMITS)
                .unwrap()
                .view()
                .entry(0)
                .unwrap()
                .payload()[0]
        }

        fn send_fixture_module(
            address: std::net::SocketAddr,
            credentials: &SessionCredentials,
            protocol_limits: ProtocolLimits,
            request_id: u64,
            module: &[u8],
        ) -> thread::JoinHandle<ReloadResult> {
            let credentials = credentials.clone();
            let module = module.to_vec();
            thread::spawn(move || {
                let mut stream = TcpStream::connect(address).unwrap();
                send_reload_command(
                    &mut stream,
                    &credentials,
                    protocol_limits,
                    request_id,
                    ModuleMetadata::new([1; 16], [2; 16], 1, 0, [request_id as u8; 32]),
                    &module,
                )
                .unwrap()
            })
        }

        fn active_counter_from_app<W: Widget + 'static>(
            app: &mut crate::HeadlessAimerApp<W>,
        ) -> u8 {
            let state = app.live_reload_state().unwrap().unwrap();
            StateBundleView::decode(&state, MODEL_LIMITS)
                .unwrap()
                .entry(0)
                .unwrap()
                .payload()[0]
        }

        fn callback_event(generation: GenerationId, sequence: u64) -> Vec<u8> {
            CallbackEvent::new(
                generation.get(),
                sequence,
                CALLBACK_ID,
                EVENT_BUTTON_PRESS,
                Version::new(1, 0),
                sequence,
                &[],
            )
            .encode(MODEL_LIMITS)
            .unwrap()
        }

        fn runtime() -> Runtime {
            Runtime::new(runtime_config())
        }

        fn runtime_config() -> RuntimeConfig {
            RuntimeConfig::new()
                .fuel_per_call(10_000_000)
                .max_module_bytes(16 * 1_024 * 1_024)
                .max_memory_pages(64)
                .max_table_elements(1_024)
                .max_call_depth(256)
        }

        fn stateful_guest_module() -> &'static [u8] {
            static MODULE: OnceLock<Vec<u8>> = OnceLock::new();
            MODULE.get_or_init(|| {
                let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .ancestors()
                    .nth(2)
                    .unwrap()
                    .to_path_buf();
                let fixture =
                    workspace.join("experiments/wasm_guest/fixtures/stateful_guest/Cargo.toml");
                let target = workspace.join("target/candidate-preparation-fixture");
                let status = Command::new(env!("CARGO"))
                    .args([
                        "build",
                        "--manifest-path",
                        fixture.to_str().unwrap(),
                        "--target",
                        "wasm32-unknown-unknown",
                        "--target-dir",
                        target.to_str().unwrap(),
                    ])
                    .status()
                    .unwrap();
                assert!(
                    status.success(),
                    "stateful WASM guest fixture failed to build"
                );
                fs::read(target.join("wasm32-unknown-unknown/debug/aimer_stateful_wasm_guest.wasm"))
                    .unwrap()
            })
        }

        fn stateful_guest_module_with_feature(feature: &str) -> Vec<u8> {
            let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .nth(2)
                .unwrap()
                .to_path_buf();
            let fixture =
                workspace.join("experiments/wasm_guest/fixtures/stateful_guest/Cargo.toml");
            let target = workspace
                .join("target/candidate-preparation-fixture")
                .join(feature);
            let status = Command::new(env!("CARGO"))
                .args([
                    "build",
                    "--manifest-path",
                    fixture.to_str().unwrap(),
                    "--features",
                    feature,
                    "--target",
                    "wasm32-unknown-unknown",
                    "--target-dir",
                    target.to_str().unwrap(),
                ])
                .status()
                .unwrap();
            assert!(
                status.success(),
                "stateful WASM guest variant {feature} failed to build"
            );
            fs::read(target.join("wasm32-unknown-unknown/debug/aimer_stateful_wasm_guest.wasm"))
                .unwrap()
        }

        fn context() -> BuildContext<'static> {
            let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
            BuildContext::new(
                aimer_canvas::FrameCanvas::new(inner),
                Default::default(),
                1.0,
                Default::default(),
                Default::default(),
                WindowHandle::headless(Default::default(), 1.0),
                dummy_async_handle(),
            )
        }

        fn dummy_async_handle() -> tokio::runtime::Handle {
            static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
            let runtime = RUNTIME.get_or_init(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
            });
            let _guard = runtime.enter();
            tokio::runtime::Handle::current()
        }
    }
    #[cfg(feature = "wasm-hot-reload")]
    mod derived_materializer_conflict {
        use aimer_anteros::PortableWidgetSchemaValidator;
        use aimer_macro::PortableWidget;
        use aimer_widget::base::{BuildContext, WindowHandle};
        use aimer_widget::portable::PortableWidgetSchema;
        use aimer_widget::{AnyElement, Widget};
        use std::marker::PhantomData;

        #[derive(PortableWidget)]
        #[portable_widget(id = "aimer_quiver.tests.ConflictingDerivedWidget", schema_only)]
        struct FirstConflict<T = ()> {
            #[portable_skip]
            marker: PhantomData<T>,
        }

        #[allow(dead_code)]
        impl FirstConflict {
            #[inline]
            fn new() -> Self {
                Self {
                    marker: PhantomData,
                }
            }
        }

        impl Widget for FirstConflict {
            fn to_element(self, ctx: &BuildContext) -> AnyElement {
                aimer_text::Text::new("first").to_element(ctx)
            }
        }

        #[derive(PortableWidget)]
        #[portable_widget(id = "aimer_quiver.tests.ConflictingDerivedWidget", schema_only)]
        struct SecondConflict<T = ()> {
            #[portable_skip]
            marker: PhantomData<T>,
        }

        #[allow(dead_code)]
        impl SecondConflict {
            #[inline]
            fn new() -> Self {
                Self {
                    marker: PhantomData,
                }
            }
        }

        impl Widget for SecondConflict {
            fn to_element(self, ctx: &BuildContext) -> AnyElement {
                aimer_text::Text::new("second").to_element(ctx)
            }
        }

        // Inline tests share one linker registry. Keep this duplicate pair out
        // of that global registry and verify that identical contracts remain
        // valid when more than one generic path publishes them.
        #[test]
        fn identical_derived_schemas_are_accepted() {
            let schemas = [
                <FirstConflict as PortableWidgetSchema>::SCHEMA,
                <SecondConflict as PortableWidgetSchema>::SCHEMA,
            ];
            assert!(PortableWidgetSchemaValidator::new(&schemas).is_ok());
        }

        #[allow(dead_code)]
        fn context() -> BuildContext<'static> {
            let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
            BuildContext::new(
                aimer_canvas::FrameCanvas::new(inner),
                Default::default(),
                1.0,
                Default::default(),
                Default::default(),
                WindowHandle::headless(Default::default(), 1.0),
                dummy_async_handle(),
            )
        }

        #[allow(dead_code)]
        fn dummy_async_handle() -> tokio::runtime::Handle {
            use std::sync::OnceLock;

            static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
            let runtime = RUNTIME.get_or_init(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
            });
            let _guard = runtime.enter();
            tokio::runtime::Handle::current()
        }
    }
    #[cfg(feature = "wasm-hot-reload")]
    mod headless_reload {
        use std::any::Any;
        use std::cell::Cell;
        use std::cell::RefCell;
        use std::rc::Rc;

        use crate::handler::{HeadlessReloadHost, ReloadCommand};
        use aimer_anteros::{
            CallbackBindingSnapshot, Generation, GenerationId, GenerationLimits,
            ReloadEventDisposition, ReloadGuest, ReloadSnapshot,
        };
        use aimer_widget::base::{BuildContext, WindowHandle};
        use aimer_widget::{
            AnyElement, Drawable, Element, EventElement, LayoutElement, Rebuildable, VisitorElement,
        };

        #[test]
        fn command_commits_only_at_the_headless_host_safe_point_and_requests_one_frame() {
            let lifecycle = Rc::new(RefCell::new(Vec::new()));
            let frames = Rc::new(RefCell::new(0));
            let requested_frames = frames.clone();
            let active = snapshot(1, "old-root", lifecycle.clone());
            let mut host: HeadlessReloadHost<RecordingGuest, _, u8> =
                HeadlessReloadHost::new(active, 4, move || *requested_frames.borrow_mut() += 1);
            let transaction = host.begin_reload();
            host.queue_command(ReloadCommand::new(
                transaction,
                snapshot(2, "new-root", lifecycle.clone()),
            ))
            .unwrap();

            assert_eq!(host.active().generation_id(), GenerationId::new(1));
            assert_eq!(host.active().root(), &"old-root");
            assert_eq!(*frames.borrow(), 0);

            let committed = host
                .process_safe_point(
                    |old, candidate| {
                        assert_eq!(old.root(), &"old-root");
                        assert_eq!(candidate.root(), &"new-root");
                        Ok::<(), ()>(())
                    },
                    |_old, _candidate| {},
                )
                .unwrap()
                .unwrap();

            assert_eq!(committed.generation_id(), GenerationId::new(2));
            assert_eq!(host.active().generation_id(), GenerationId::new(2));
            assert_eq!(host.active().root(), &"new-root");
            assert_eq!(*frames.borrow(), 1);
            assert!(
                host.process_safe_point(
                    |_old, _candidate| Ok::<(), ()>(()),
                    |_old, _candidate| {},
                )
                .unwrap()
                .is_none()
            );
            assert_eq!(*frames.borrow(), 1);
            assert_eq!(
                lifecycle.borrow().as_slice(),
                [LifecycleEvent::Activated(2), LifecycleEvent::Retired(1)]
            );
        }

        #[test]
        fn safe_point_rejection_keeps_the_old_snapshot_and_releases_queued_events() {
            let lifecycle = Rc::new(RefCell::new(Vec::new()));
            let active = snapshot(1, "old-root", lifecycle.clone());
            let mut host = HeadlessReloadHost::new(active, 2, || {});
            let transaction = host.begin_reload();
            host.queue_command(ReloadCommand::new(
                transaction,
                snapshot(2, "candidate-root", lifecycle.clone()),
            ))
            .unwrap();
            assert_eq!(
                host.route_event("first").unwrap(),
                ReloadEventDisposition::Queued
            );
            assert_eq!(
                host.route_event("second").unwrap(),
                ReloadEventDisposition::Queued
            );

            let rejection = host
                .process_safe_point(
                    |_old, _candidate| Err("reconciliation rejected"),
                    |_old, _candidate| unreachable!(),
                )
                .unwrap_err();

            assert_eq!(
                rejection.preflight_error(),
                Some(&"reconciliation rejected")
            );
            assert_eq!(rejection.replay().unwrap().as_slice(), &["first", "second"]);
            assert_eq!(host.active().generation_id(), GenerationId::new(1));
            assert_eq!(host.active().root(), &"old-root");
            assert_eq!(lifecycle.borrow().as_slice(), [LifecycleEvent::Retired(2)]);
        }

        #[test]
        fn repeated_commit_and_rejection_cycles_leave_no_pending_candidate() {
            const CYCLES: u64 = 256;

            let lifecycle = Rc::new(RefCell::new(Vec::new()));
            let active = snapshot(1, "root-1", lifecycle.clone());
            let mut host: HeadlessReloadHost<RecordingGuest, _, ()> =
                HeadlessReloadHost::new(active, 4, || {});
            let mut active_generation = 1;
            let mut expected_lifecycle = Vec::with_capacity(CYCLES as usize * 2);

            for generation in 2..=CYCLES + 1 {
                let transaction = host.begin_reload();
                assert!(!host.has_pending_command());
                host.queue_command(ReloadCommand::new(
                    transaction,
                    snapshot(generation, "candidate-root", lifecycle.clone()),
                ))
                .unwrap();
                assert!(host.has_pending_command());

                if generation % 3 == 0 {
                    host.process_safe_point(
                        |_old, _candidate| Err::<(), _>("injected pre-commit rejection"),
                        |_old, _candidate| unreachable!(),
                    )
                    .unwrap_err();
                    expected_lifecycle.push(LifecycleEvent::Retired(generation));
                } else {
                    host.process_safe_point(
                        |_old, _candidate| Ok::<(), ()>(()),
                        |_old, _candidate| {},
                    )
                    .unwrap()
                    .unwrap();
                    expected_lifecycle.push(LifecycleEvent::Activated(generation));
                    expected_lifecycle.push(LifecycleEvent::Retired(active_generation));
                    active_generation = generation;
                }

                assert!(!host.has_pending_command());
                assert_eq!(
                    host.active().generation_id(),
                    GenerationId::new(active_generation)
                );
            }

            assert_eq!(lifecycle.borrow().as_slice(), expected_lifecycle);
        }

        #[test]
        fn element_safe_point_carries_native_identity_and_state_only_at_commit() {
            let lifecycle = Rc::new(RefCell::new(Vec::new()));
            let old_root = StatefulRoot::new(17);
            let old_id = old_root.id();
            let candidate_root = StatefulRoot::new(0);
            let candidate_id = candidate_root.id();
            let active = element_snapshot(1, old_root, lifecycle.clone());
            let mut host: HeadlessReloadHost<RecordingGuest, AnyElement, ()> =
                HeadlessReloadHost::new(active, 1, || {});
            let transaction = host.begin_reload();
            host.queue_command(ReloadCommand::new(
                transaction,
                element_snapshot(2, candidate_root, lifecycle),
            ))
            .unwrap();

            assert_eq!(host.active().root().id(), old_id);
            assert_ne!(candidate_id, old_id);

            host.process_element_safe_point(&context())
                .unwrap()
                .unwrap();

            assert_eq!(host.active().root().id(), old_id);
            assert_eq!(root_state(host.active().root()), 17);
        }

        fn element_snapshot(
            generation_id: u64,
            root: AnyElement,
            lifecycle: Rc<RefCell<Vec<LifecycleEvent>>>,
        ) -> ReloadSnapshot<RecordingGuest, AnyElement> {
            ReloadSnapshot::new(
                Generation::with_guest(
                    GenerationId::new(generation_id),
                    CallbackBindingSnapshot::empty(),
                    aimer_venus::LocalScheduler::new(),
                    GenerationLimits::new(0),
                    RecordingGuest {
                        generation_id,
                        lifecycle,
                    },
                ),
                root,
            )
        }

        struct StatefulRoot {
            state: Cell<u32>,
        }

        impl StatefulRoot {
            fn new(state: u32) -> AnyElement {
                Self {
                    state: Cell::new(state),
                }
                .boxed()
            }
        }

        impl VisitorElement for StatefulRoot {
            fn debug_name(&self) -> &'static str {
                "StatefulRoot"
            }
        }

        impl EventElement for StatefulRoot {}
        impl LayoutElement for StatefulRoot {}
        impl Drawable for StatefulRoot {
            fn update(&self, _ctx: &BuildContext) {}
        }
        impl Rebuildable for StatefulRoot {
            fn option_any(&self) -> Option<&dyn Any> {
                Some(self)
            }

            fn adopt_runtime_state_from(&self, old: &dyn Element) {
                let old = old
                    .option_any()
                    .and_then(|value| value.downcast_ref::<Self>())
                    .unwrap();
                self.state.set(old.state.get());
            }
        }

        fn root_state(root: &AnyElement) -> u32 {
            root.option_any()
                .and_then(|value| value.downcast_ref::<StatefulRoot>())
                .unwrap()
                .state
                .get()
        }

        #[cfg(not(target_arch = "wasm32"))]
        fn dummy_async_handle() -> tokio::runtime::Handle {
            use std::sync::OnceLock;

            static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
            let runtime = RUNTIME.get_or_init(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
            });
            let _guard = runtime.enter();
            tokio::runtime::Handle::current()
        }

        fn context() -> BuildContext<'static> {
            let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
            BuildContext::new(
                aimer_canvas::FrameCanvas::new(inner),
                Default::default(),
                1.0,
                Default::default(),
                Default::default(),
                WindowHandle::headless(Default::default(), 1.0),
                #[cfg(not(target_arch = "wasm32"))]
                dummy_async_handle(),
            )
        }

        fn snapshot(
            generation_id: u64,
            root: &'static str,
            lifecycle: Rc<RefCell<Vec<LifecycleEvent>>>,
        ) -> ReloadSnapshot<RecordingGuest, &'static str> {
            ReloadSnapshot::new(
                Generation::with_guest(
                    GenerationId::new(generation_id),
                    CallbackBindingSnapshot::empty(),
                    aimer_venus::LocalScheduler::new(),
                    GenerationLimits::new(0),
                    RecordingGuest {
                        generation_id,
                        lifecycle,
                    },
                ),
                root,
            )
        }

        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        enum LifecycleEvent {
            Activated(u64),
            Retired(u64),
        }

        struct RecordingGuest {
            generation_id: u64,
            lifecycle: Rc<RefCell<Vec<LifecycleEvent>>>,
        }

        impl ReloadGuest for RecordingGuest {
            fn activate(&mut self) {
                self.lifecycle
                    .borrow_mut()
                    .push(LifecycleEvent::Activated(self.generation_id));
            }

            fn retire(&mut self) {
                self.lifecycle
                    .borrow_mut()
                    .push(LifecycleEvent::Retired(self.generation_id));
            }
        }
    }
    #[cfg(feature = "wasm-hot-reload")]
    mod reload_conformance {
        use std::any::Any;
        use std::cell::{Cell, RefCell};
        use std::net::{SocketAddr, TcpStream};
        use std::rc::Rc;
        use std::sync::mpsc::TryRecvError;
        use std::thread::{self, JoinHandle};
        use std::time::Duration;

        use crate::handler::{HeadlessReloadHost, ReloadCommand};
        use crate::hot_reload::{
            EVENT_BUTTON_PRESS, PROPERTY_TEXT_CONTENT, PendingProtocolReload, ProtocolReloadInbox,
            WIDGET_BUTTON, WIDGET_COLUMN, WIDGET_TEXT, materialize_aimer_widget_tree,
            protocol_reload_stage, reload_command_bridge,
        };
        use crate::reload_protocol::{
            ModuleMetadata, ProtocolLimits, ReloadConnectionOutcome, ReloadResult, ReloadStage,
            SessionCredentials, query_reload_result, send_reload_command,
        };
        use crate::reload_server::{ListenerSecurity, ReloadCommandListener};
        use aimer_anteros::{
            CallbackBinding, CallbackBindingError, CallbackBindingSnapshot, CallbackEvent,
            Generation, GenerationId, GenerationLimits, GenerationResource, GenerationResourceKind,
            ModelLimits, PreparedStateTransfer, PropertyValue, ReloadEventDisposition, ReloadGuest,
            ReloadReplay, ReloadSnapshot, ReloadStage as RuntimeReloadStage, StableId128,
            StateBundle, StateBundleView, StateEntry, StateMigration, StateMigrationFailure,
            StatePolicy, StateTransferCoordinator, Version, WidgetDocument, WidgetDocumentView,
            WidgetNode, WidgetProperty,
        };
        use aimer_widget::base::{BuildContext, WindowHandle};
        use aimer_widget::{
            AnyElement, Drawable, Element, EventElement, LayoutElement, Rebuildable, VisitorElement,
        };

        const TIMEOUT: Duration = Duration::from_secs(10);
        const MODEL_LIMITS: ModelLimits =
            ModelLimits::new(8_192, 64, 256, 256).max_widget_depth(16);
        const WIDGET_SCHEMA: Version = Version::new(1, 0);
        const STATE_V1: Version = Version::new(1, 0);
        const STATE_V2: Version = Version::new(2, 0);
        const MAX_CALLBACK_BINDINGS: u32 = 8;
        const MAX_QUEUED_EVENTS: usize = 4;
        const MAX_GENERATION_RESOURCES: u32 = 4;
        const MIGRATION_FUEL: u64 = 16;
        const COUNTER_MIGRATION_FUEL: u64 = 4;
        const BOOTSTRAP_GENERATION: u64 = 0;

        const APPLICATION_ID: StableId128 = StableId128::from_bytes([0x0A; 16]);
        const COUNTER_STATE: StableId128 = StableId128::from_bytes([0xC0; 16]);
        const DRAFT_STATE: StableId128 = StableId128::from_bytes([0xD0; 16]);
        const COUNTER_SCHEMA: StableId128 = StableId128::from_bytes([0xC5; 16]);
        const DRAFT_SCHEMA: StableId128 = StableId128::from_bytes([0xD5; 16]);
        const BUTTON_KEY: StableId128 = StableId128::from_bytes([0xB0; 16]);
        const PRESS_CALLBACK_V1: StableId128 = StableId128::from_bytes([0x51; 16]);
        const PRESS_CALLBACK_V2: StableId128 = StableId128::from_bytes([0x52; 16]);

        const ENVELOPE_ERROR: u32 = 0x9001;
        const VALIDATION_ERROR: u32 = 0x9002;
        const STATE_TRANSFER_ERROR: u32 = 0x9003;
        const RECONCILIATION_ERROR: u32 = 0x9004;

        #[test]
        fn first_authenticated_module_commits_the_first_generation_and_requests_one_frame() {
            let mut app = ConformanceHost::new();
            let (address, inbox, server) = spawn_command_listener(BOOTSTRAP_GENERATION, 1);
            let client = spawn_command_client(
                address,
                1,
                guest_module("first", PRESS_CALLBACK_V1, counter_state(1, STATE_V1, &[7])),
            );

            let pending = inbox.recv_timeout(TIMEOUT).unwrap();
            assert_eq!(pending.command().request_id(), 1);
            assert_eq!(app.active_generation(), BOOTSTRAP_GENERATION);
            let served = app.serve(pending);

            assert_eq!(
                served.result,
                ReloadResult::Committed {
                    active_generation: 1,
                    reset_state_entries: 0,
                    cleanup_warnings: 0,
                }
            );
            assert_eq!(client.join().unwrap(), served.result);
            assert_eq!(command_results(server), [served.result]);
            assert_eq!(app.active_generation(), 1);
            assert_eq!(inbox.active_generation(), 1);
            assert_eq!(app.frames(), 1);
            assert!(app.process_idle_safe_point().is_none());
            assert_eq!(app.frames(), 1);
            assert_eq!(app.lifecycle(), ["activated:1", "retired:0"]);
        }

        #[test]
        fn second_commit_preserves_guest_state_and_native_element_runtime_state() {
            let mut app = ConformanceHost::new();
            let (address, inbox, server) = spawn_command_listener(BOOTSTRAP_GENERATION, 2);
            let installed = spawn_command_client(
                address,
                1,
                guest_module("first", PRESS_CALLBACK_V1, counter_state(1, STATE_V1, &[7])),
            );
            app.serve(inbox.recv_timeout(TIMEOUT).unwrap());
            installed.join().unwrap();
            app.set_guest_state(counter_state(1, STATE_V1, &[42]));
            app.set_element_counter(17);

            let reloaded = spawn_command_client(
                address,
                2,
                guest_module(
                    "second",
                    PRESS_CALLBACK_V1,
                    counter_state(2, STATE_V1, &[0]),
                ),
            );
            let served = app.serve(inbox.recv_timeout(TIMEOUT).unwrap());

            assert_eq!(
                served.result,
                ReloadResult::Committed {
                    active_generation: 2,
                    reset_state_entries: 0,
                    cleanup_warnings: 0,
                }
            );
            assert_eq!(reloaded.join().unwrap(), served.result);
            assert_eq!(command_results(server).len(), 2);
            assert_eq!(served.report().preserved_entries(), 1);
            assert_eq!(app.guest_payload(COUNTER_STATE), [42]);
            assert_eq!(app.element_counter(), 17);
            assert_eq!(app.active_generation(), 2);
            assert_eq!(
                app.lifecycle(),
                ["activated:1", "retired:0", "activated:2", "retired:1"]
            );
            assert_eq!(app.released(), ["released:0", "released:1"]);
        }

        #[test]
        fn schema_upgrading_generation_migrates_required_state_and_reports_reset_safe_entries() {
            let mut app = ConformanceHost::new();
            app.register_counter_migration();
            let (address, inbox, server) = spawn_command_listener(BOOTSTRAP_GENERATION, 2);
            let installed = spawn_command_client(
                address,
                1,
                guest_module(
                    "first",
                    PRESS_CALLBACK_V1,
                    counter_and_draft_state(1, STATE_V1, &[7], STATE_V1, b"empty"),
                ),
            );
            app.serve(inbox.recv_timeout(TIMEOUT).unwrap());
            installed.join().unwrap();
            app.set_guest_state(counter_and_draft_state(
                1,
                STATE_V1,
                &[9],
                STATE_V1,
                b"typed",
            ));

            let migrated = spawn_command_client(
                address,
                2,
                guest_module(
                    "second",
                    PRESS_CALLBACK_V1,
                    counter_and_draft_state(2, STATE_V2, &[0, 0], STATE_V2, b"fresh"),
                ),
            );
            let served = app.serve(inbox.recv_timeout(TIMEOUT).unwrap());

            assert_eq!(
                served.result,
                ReloadResult::Committed {
                    active_generation: 2,
                    reset_state_entries: 1,
                    cleanup_warnings: 0,
                }
            );
            assert_eq!(migrated.join().unwrap(), served.result);
            assert_eq!(command_results(server).len(), 2);
            assert_eq!(served.report().migrated_state_ids(), [COUNTER_STATE]);
            assert_eq!(served.report().reset_state_ids(), [DRAFT_STATE]);
            assert_eq!(
                served.report().migration_fuel_consumed(),
                COUNTER_MIGRATION_FUEL
            );
            assert_eq!(app.guest_payload(COUNTER_STATE), [9, 0]);
            assert_eq!(app.guest_payload(DRAFT_STATE), *b"fresh");
            assert_eq!(app.active_generation(), 2);
        }

        #[test]
        fn rebound_callback_identities_dispatch_to_the_new_generation_and_retired_events_are_rejected()
         {
            let mut app = ConformanceHost::new();
            let (address, inbox, server) = spawn_command_listener(BOOTSTRAP_GENERATION, 2);
            let installed = spawn_command_client(
                address,
                1,
                guest_module("first", PRESS_CALLBACK_V1, counter_state(1, STATE_V1, &[0])),
            );
            app.serve(inbox.recv_timeout(TIMEOUT).unwrap());
            installed.join().unwrap();
            app.dispatch_press(1, 1, PRESS_CALLBACK_V1).unwrap();

            let rebound = spawn_command_client(
                address,
                2,
                guest_module(
                    "second",
                    PRESS_CALLBACK_V2,
                    counter_state(2, STATE_V1, &[0]),
                ),
            );
            app.serve(inbox.recv_timeout(TIMEOUT).unwrap());
            rebound.join().unwrap();
            command_results(server);

            app.dispatch_press(2, 1, PRESS_CALLBACK_V2).unwrap();
            assert_eq!(
                app.dispatch_press(1, 2, PRESS_CALLBACK_V1).unwrap_err(),
                CallbackBindingError::GenerationMismatch {
                    expected: GenerationId::new(2),
                    actual: GenerationId::new(1),
                }
            );
            assert_eq!(
                app.dispatch_press(2, 2, PRESS_CALLBACK_V1).unwrap_err(),
                CallbackBindingError::UnknownCallback {
                    callback_id: PRESS_CALLBACK_V1,
                }
            );
            assert_eq!(
                app.dispatched(),
                [(1, PRESS_CALLBACK_V1), (2, PRESS_CALLBACK_V2)]
            );
        }

        #[test]
        fn rejected_module_keeps_the_committed_generation_active_and_a_later_module_still_commits()
        {
            let mut app = ConformanceHost::new();
            let (address, inbox, server) = spawn_command_listener(BOOTSTRAP_GENERATION, 3);
            let installed = spawn_command_client(
                address,
                1,
                guest_module("first", PRESS_CALLBACK_V1, counter_state(1, STATE_V1, &[7])),
            );
            app.serve(inbox.recv_timeout(TIMEOUT).unwrap());
            installed.join().unwrap();
            app.set_element_counter(5);

            let rejected = spawn_command_client(address, 2, uncompilable_module());
            let rejection = app.serve(inbox.recv_timeout(TIMEOUT).unwrap());

            assert!(matches!(
                &rejection.result,
                ReloadResult::Rejected {
                    stage: ReloadStage::Validation,
                    error_code: VALIDATION_ERROR,
                    active_generation: 1,
                    ..
                }
            ));
            assert_eq!(rejected.join().unwrap(), rejection.result);
            assert_eq!(app.active_generation(), 1);
            assert_eq!(app.element_counter(), 5);
            assert_eq!(app.frames(), 1);
            assert_eq!(app.released(), ["released:0"]);

            let accepted = spawn_command_client(
                address,
                3,
                guest_module("third", PRESS_CALLBACK_V1, counter_state(3, STATE_V1, &[0])),
            );
            let served = app.serve(inbox.recv_timeout(TIMEOUT).unwrap());

            assert_eq!(
                served.result,
                ReloadResult::Committed {
                    active_generation: 2,
                    reset_state_entries: 0,
                    cleanup_warnings: 0,
                }
            );
            assert_eq!(accepted.join().unwrap(), served.result);
            assert_eq!(command_results(server).len(), 3);
            assert_eq!(app.guest_payload(COUNTER_STATE), [7]);
            assert_eq!(app.element_counter(), 5);
            assert_eq!(app.frames(), 2);
        }

        #[test]
        fn pre_commit_failure_maps_to_its_protocol_stage_and_releases_the_candidate_once() {
            let mut app = ConformanceHost::new();
            let (address, inbox, server) = spawn_command_listener(BOOTSTRAP_GENERATION, 2);
            let installed = spawn_command_client(
                address,
                1,
                guest_module("first", PRESS_CALLBACK_V1, counter_state(1, STATE_V1, &[7])),
            );
            let first = app.serve(inbox.recv_timeout(TIMEOUT).unwrap());
            installed.join().unwrap();
            app.set_element_counter(11);

            let rolled_back = spawn_command_client(
                address,
                2,
                guest_module(
                    "second",
                    PRESS_CALLBACK_V2,
                    counter_state(2, STATE_V1, &[0]),
                ),
            );
            let rejection = app.serve_with_rejected_pre_commit(
                inbox.recv_timeout(TIMEOUT).unwrap(),
                &["press", "scroll"],
            );

            let result = rolled_back.join().unwrap();
            assert_eq!(
                result,
                ReloadResult::Rejected {
                    stage: protocol_reload_stage(RuntimeReloadStage::PrepareReconciliation),
                    error_code: RECONCILIATION_ERROR,
                    active_generation: 1,
                    diagnostic: rejection.diagnostic.clone(),
                }
            );
            assert!(matches!(
                &result,
                ReloadResult::Rejected {
                    stage: ReloadStage::Reconciliation,
                    ..
                }
            ));
            assert_eq!(command_results(server), [first.result, result]);
            assert_eq!(rejection.replay.as_slice(), ["press", "scroll"]);
            assert_eq!(app.active_generation(), 1);
            assert_eq!(app.element_counter(), 11);
            assert_eq!(app.guest_payload(COUNTER_STATE), [7]);
            assert_eq!(app.frames(), 1);
            assert_eq!(app.released(), ["released:0", "released:2"]);
            assert_eq!(app.lifecycle(), ["activated:1", "retired:0", "retired:2"]);
        }

        #[test]
        fn reconnecting_client_recovers_the_same_terminal_result_without_reexecuting_it() {
            let mut app = ConformanceHost::new();
            let (address, inbox, server) = spawn_command_listener(BOOTSTRAP_GENERATION, 2);
            let module = guest_module("first", PRESS_CALLBACK_V1, counter_state(1, STATE_V1, &[7]));
            let client = thread::spawn(move || {
                let mut stream = connect_loopback_client(address);
                let committed = send_reload_command(
                    &mut stream,
                    &credentials(),
                    protocol_limits(),
                    7,
                    module_metadata(),
                    &module,
                )
                .unwrap();
                drop(stream);
                let mut reconnected = connect_loopback_client(address);
                let recovered =
                    query_reload_result(&mut reconnected, &credentials(), protocol_limits(), 7)
                        .unwrap();
                (committed, recovered)
            });

            let served = app.serve(inbox.recv_timeout(TIMEOUT).unwrap());
            let (committed, recovered) = client.join().unwrap();

            assert_eq!(committed, served.result);
            assert_eq!(recovered, Some(served.result.clone()));
            assert!(matches!(
                inbox.try_recv(),
                Err(TryRecvError::Empty | TryRecvError::Disconnected)
            ));
            assert_eq!(
                server.join().unwrap(),
                [
                    ReloadConnectionOutcome::Command(served.result.clone()),
                    ReloadConnectionOutcome::Query(Some(served.result)),
                ]
            );
            assert_eq!(app.prepared_candidates(), 1);
            assert_eq!(app.active_generation(), 1);
            assert_eq!(app.frames(), 1);
        }

        #[test]
        fn repeated_authenticated_reconnects_recover_one_result_without_reexecution() {
            const RECONNECTS: usize = 64;

            let mut app = ConformanceHost::new();
            let (address, inbox, server) =
                spawn_command_listener(BOOTSTRAP_GENERATION, RECONNECTS + 1);
            let module = guest_module("first", PRESS_CALLBACK_V1, counter_state(1, STATE_V1, &[7]));
            let client = thread::spawn(move || {
                let mut stream = connect_loopback_client(address);
                let committed = send_reload_command(
                    &mut stream,
                    &credentials(),
                    protocol_limits(),
                    7,
                    module_metadata(),
                    &module,
                )
                .unwrap();
                drop(stream);

                for _ in 0..RECONNECTS {
                    let mut reconnected = connect_loopback_client(address);
                    assert_eq!(
                        query_reload_result(&mut reconnected, &credentials(), protocol_limits(), 7)
                            .unwrap(),
                        Some(committed.clone())
                    );
                }
                committed
            });

            let served = app.serve(inbox.recv_timeout(TIMEOUT).unwrap());
            let committed = client.join().unwrap();
            let outcomes = server.join().unwrap();

            assert_eq!(committed, served.result);
            assert_eq!(outcomes.len(), RECONNECTS + 1);
            assert_eq!(
                outcomes
                    .iter()
                    .filter(|outcome| matches!(outcome, ReloadConnectionOutcome::Command(_)))
                    .count(),
                1
            );
            assert!(outcomes.iter().skip(1).all(|outcome| {
                matches!(outcome, ReloadConnectionOutcome::Query(Some(result)) if result == &served.result)
            }));
            assert_eq!(app.prepared_candidates(), 1);
            assert_eq!(app.active_generation(), 1);
            assert_eq!(app.frames(), 1);
        }

        #[test]
        fn dropping_the_host_reports_a_stable_terminal_status_and_leaves_no_pending_candidate() {
            let mut app = ConformanceHost::new();
            let (address, inbox, server) = spawn_command_listener(BOOTSTRAP_GENERATION, 2);
            let cancelled = spawn_command_client(
                address,
                1,
                guest_module("first", PRESS_CALLBACK_V1, counter_state(1, STATE_V1, &[7])),
            );

            let pending = inbox.recv_timeout(TIMEOUT).unwrap();
            app.stage_candidate(pending.command().module());
            let lifecycle = app.lifecycle_log();
            let released = app.released_log();
            drop(app);
            drop(pending);

            let result = cancelled.join().unwrap();
            assert!(matches!(
                &result,
                ReloadResult::Rejected {
                    stage: ReloadStage::Cancellation,
                    active_generation: BOOTSTRAP_GENERATION,
                    ..
                }
            ));
            assert_eq!(lifecycle.borrow().as_slice(), ["retired:0", "retired:1"]);
            assert_eq!(released.borrow().as_slice(), ["released:0", "released:1"]);

            drop(inbox);
            let disconnected = spawn_command_client(
                address,
                2,
                guest_module(
                    "second",
                    PRESS_CALLBACK_V1,
                    counter_state(2, STATE_V1, &[0]),
                ),
            );
            let after_exit = disconnected.join().unwrap();

            assert!(matches!(
                &after_exit,
                ReloadResult::Rejected {
                    stage: ReloadStage::CommitWait,
                    active_generation: BOOTSTRAP_GENERATION,
                    ..
                }
            ));
            assert_eq!(command_results(server), [result, after_exit]);
        }

        struct ConformanceHost {
            host: HeadlessReloadHost<FakeGuest, AnyElement, &'static str>,
            transfer: StateTransferCoordinator,
            frames: Rc<Cell<u32>>,
            lifecycle: Rc<RefCell<Vec<String>>>,
            dispatched: Rc<RefCell<Vec<(u64, StableId128)>>>,
            released: Rc<RefCell<Vec<String>>>,
            next_generation: u64,
        }

        impl ConformanceHost {
            fn new() -> Self {
                let lifecycle = Rc::new(RefCell::new(Vec::new()));
                let dispatched = Rc::new(RefCell::new(Vec::new()));
                let released = Rc::new(RefCell::new(Vec::new()));
                let frames = Rc::new(Cell::new(0));
                let requested_frames = Rc::clone(&frames);
                let mut generation = Generation::with_guest(
                    GenerationId::new(BOOTSTRAP_GENERATION),
                    CallbackBindingSnapshot::empty(),
                    aimer_venus::LocalScheduler::new(),
                    GenerationLimits::new(MAX_GENERATION_RESOURCES),
                    FakeGuest::new(
                        BOOTSTRAP_GENERATION,
                        bootstrap_state(),
                        Rc::clone(&dispatched),
                        Rc::clone(&lifecycle),
                    ),
                );
                generation
                    .register_resource(
                        GenerationResourceKind::Timer,
                        TrackedResource::new(BOOTSTRAP_GENERATION, Rc::clone(&released)),
                    )
                    .unwrap();
                Self {
                    host: HeadlessReloadHost::new(
                        ReloadSnapshot::new(generation, CounterRoot::new(0, None)),
                        MAX_QUEUED_EVENTS,
                        move || requested_frames.set(requested_frames.get() + 1),
                    ),
                    transfer: StateTransferCoordinator::new()
                        .model_limits(MODEL_LIMITS)
                        .migration_fuel(MIGRATION_FUEL),
                    frames,
                    lifecycle,
                    dispatched,
                    released,
                    next_generation: BOOTSTRAP_GENERATION + 1,
                }
            }

            fn register_counter_migration(&mut self) {
                self.transfer
                    .register_migration(StateMigration::new(
                        COUNTER_STATE,
                        COUNTER_SCHEMA,
                        STATE_V1,
                        COUNTER_SCHEMA,
                        STATE_V2,
                        COUNTER_MIGRATION_FUEL,
                        widen_counter,
                    ))
                    .unwrap();
            }

            fn serve(&mut self, pending: PendingProtocolReload) -> ServedReload {
                let transaction = self.host.begin_reload();
                match self.prepare_candidate(pending.command().module()) {
                    Ok(prepared) => {
                        let reset_state_entries =
                            prepared.transfer.report().reset_state_ids().len() as u32;
                        self.host
                            .queue_command(ReloadCommand::new(transaction, prepared.snapshot))
                            .unwrap();
                        let commit = self
                            .host
                            .process_element_safe_point(&context())
                            .unwrap()
                            .unwrap();
                        let result = ReloadResult::Committed {
                            active_generation: commit.generation_id().get(),
                            reset_state_entries,
                            cleanup_warnings: 0,
                        };
                        pending
                            .complete_commit(&commit, reset_state_entries, 0)
                            .unwrap();
                        ServedReload {
                            result,
                            transfer: Some(prepared.transfer),
                        }
                    }
                    Err(failure) => {
                        self.host.rollback(transaction).unwrap();
                        let active_generation = self.host.active().generation_id().get();
                        let result = ReloadResult::Rejected {
                            stage: protocol_reload_stage(failure.stage),
                            error_code: failure.error_code,
                            active_generation,
                            diagnostic: failure.diagnostic.clone(),
                        };
                        pending
                            .complete_rejection(
                                failure.stage,
                                failure.error_code,
                                active_generation,
                                failure.diagnostic,
                            )
                            .unwrap();
                        ServedReload {
                            result,
                            transfer: None,
                        }
                    }
                }
            }

            fn serve_with_rejected_pre_commit(
                &mut self,
                pending: PendingProtocolReload,
                events: &[&'static str],
            ) -> RejectedReload {
                let transaction = self.host.begin_reload();
                let prepared = self.prepare_candidate(pending.command().module()).unwrap();
                for event in events {
                    assert_eq!(
                        self.host.route_event(event).unwrap(),
                        ReloadEventDisposition::Queued
                    );
                }
                self.host
                    .queue_command(ReloadCommand::new(transaction, prepared.snapshot))
                    .unwrap();
                let error = self
                    .host
                    .process_safe_point(
                        |_active, _candidate| {
                            Err("native reconciliation planning rejected the candidate")
                        },
                        |_active, _candidate| {
                            unreachable!("commit ran after a rejected pre-commit stage")
                        },
                    )
                    .unwrap_err();
                let diagnostic = (*error.preflight_error().unwrap()).to_owned();
                let replay = error.into_replay().unwrap();
                let active_generation = self.host.active().generation_id().get();
                pending
                    .complete_rejection(
                        RuntimeReloadStage::PrepareReconciliation,
                        RECONCILIATION_ERROR,
                        active_generation,
                        diagnostic.clone(),
                    )
                    .unwrap();
                RejectedReload { diagnostic, replay }
            }

            fn stage_candidate(&mut self, module: &[u8]) {
                let transaction = self.host.begin_reload();
                let prepared = self.prepare_candidate(module).unwrap();
                self.host
                    .queue_command(ReloadCommand::new(transaction, prepared.snapshot))
                    .unwrap();
            }

            fn prepare_candidate(
                &mut self,
                module: &[u8],
            ) -> Result<PreparedCandidate, CandidateFailure> {
                let module = GuestModule::decode(module).ok_or_else(|| {
                    CandidateFailure::new(
                        RuntimeReloadStage::Preflight,
                        ENVELOPE_ERROR,
                        "module envelope is not an Aimer guest module",
                    )
                })?;
                let generation_id = self.next_generation;
                let dispatched = Rc::clone(&self.dispatched);
                let root = materialize_aimer_widget_tree(
                    &module.widget_image,
                    MODEL_LIMITS,
                    &context(),
                    move |callback_id| {
                        dispatched.borrow_mut().push((generation_id, callback_id));
                    },
                )
                .map_err(|error| {
                    CandidateFailure::new(RuntimeReloadStage::Validate, VALIDATION_ERROR, error)
                })?;
                let document = WidgetDocumentView::decode(&module.widget_image, MODEL_LIMITS)
                    .map_err(|error| {
                        CandidateFailure::new(RuntimeReloadStage::Validate, VALIDATION_ERROR, error)
                    })?;
                let callbacks =
                    CallbackBindingSnapshot::from_document(&document, MAX_CALLBACK_BINDINGS)
                        .map_err(|error| {
                            CandidateFailure::new(
                                RuntimeReloadStage::Validate,
                                VALIDATION_ERROR,
                                error,
                            )
                        })?;
                let transfer = self
                    .transfer
                    .prepare(
                        self.host.active().generation().guest().state(),
                        &module.default_state,
                    )
                    .map_err(|error| {
                        CandidateFailure::new(
                            RuntimeReloadStage::MigrateState,
                            STATE_TRANSFER_ERROR,
                            error,
                        )
                    })?;
                let mut generation = Generation::with_guest(
                    GenerationId::new(generation_id),
                    callbacks,
                    aimer_venus::LocalScheduler::new(),
                    GenerationLimits::new(MAX_GENERATION_RESOURCES),
                    FakeGuest::new(
                        generation_id,
                        transfer.as_bytes().to_vec(),
                        Rc::clone(&self.dispatched),
                        Rc::clone(&self.lifecycle),
                    ),
                );
                generation
                    .register_resource(
                        GenerationResourceKind::Timer,
                        TrackedResource::new(generation_id, Rc::clone(&self.released)),
                    )
                    .unwrap();
                self.next_generation += 1;
                Ok(PreparedCandidate {
                    snapshot: ReloadSnapshot::new(generation, CounterRoot::new(0, Some(root))),
                    transfer,
                })
            }

            fn dispatch_press(
                &mut self,
                generation_id: u64,
                event_sequence: u64,
                callback_id: StableId128,
            ) -> Result<(), CallbackBindingError> {
                let event = CallbackEvent::new(
                    generation_id,
                    event_sequence,
                    callback_id,
                    EVENT_BUTTON_PRESS,
                    WIDGET_SCHEMA,
                    event_sequence,
                    &[],
                )
                .widget_key(BUTTON_KEY)
                .encode(MODEL_LIMITS)
                .unwrap();
                let active = self.host.active_mut();
                let dispatched = active
                    .generation_mut()
                    .validate_event(&event, MODEL_LIMITS)?
                    .callback_id();
                active.generation_mut().guest_mut().dispatch(dispatched);
                Ok(())
            }

            fn process_idle_safe_point(&mut self) -> Option<u64> {
                self.host
                    .process_element_safe_point(&context())
                    .unwrap()
                    .map(|commit| commit.generation_id().get())
            }

            fn active_generation(&self) -> u64 {
                self.host.active().generation_id().get()
            }

            fn frames(&self) -> u32 {
                self.frames.get()
            }

            fn prepared_candidates(&self) -> u64 {
                self.next_generation - BOOTSTRAP_GENERATION - 1
            }

            fn set_guest_state(&mut self, state: Vec<u8>) {
                self.host
                    .active_mut()
                    .generation_mut()
                    .guest_mut()
                    .set_state(state);
            }

            fn guest_payload(&self, state_id: StableId128) -> Vec<u8> {
                let state = self.host.active().generation().guest().state();
                StateBundleView::decode(state, MODEL_LIMITS)
                    .unwrap()
                    .entries()
                    .find(|entry| entry.state_id() == state_id)
                    .unwrap()
                    .payload()
                    .to_vec()
            }

            fn element_counter(&self) -> u32 {
                CounterRoot::of(self.host.active().root()).counter.get()
            }

            fn set_element_counter(&mut self, value: u32) {
                CounterRoot::of(self.host.active().root())
                    .counter
                    .set(value);
            }

            fn lifecycle(&self) -> Vec<String> {
                self.lifecycle.borrow().clone()
            }

            fn lifecycle_log(&self) -> Rc<RefCell<Vec<String>>> {
                Rc::clone(&self.lifecycle)
            }

            fn released(&self) -> Vec<String> {
                self.released.borrow().clone()
            }

            fn released_log(&self) -> Rc<RefCell<Vec<String>>> {
                Rc::clone(&self.released)
            }

            fn dispatched(&self) -> Vec<(u64, StableId128)> {
                self.dispatched.borrow().clone()
            }
        }

        struct PreparedCandidate {
            snapshot: ReloadSnapshot<FakeGuest, AnyElement>,
            transfer: PreparedStateTransfer,
        }

        struct ServedReload {
            result: ReloadResult,
            transfer: Option<PreparedStateTransfer>,
        }

        impl ServedReload {
            fn report(&self) -> &aimer_anteros::StateTransferReport {
                self.transfer.as_ref().unwrap().report()
            }
        }

        struct RejectedReload {
            diagnostic: String,
            replay: ReloadReplay<&'static str>,
        }

        #[derive(Debug)]
        struct CandidateFailure {
            stage: RuntimeReloadStage,
            error_code: u32,
            diagnostic: String,
        }

        impl CandidateFailure {
            fn new(stage: RuntimeReloadStage, error_code: u32, diagnostic: impl ToString) -> Self {
                Self {
                    stage,
                    error_code,
                    diagnostic: diagnostic.to_string(),
                }
            }
        }

        struct FakeGuest {
            generation_id: u64,
            state: Vec<u8>,
            dispatched: Rc<RefCell<Vec<(u64, StableId128)>>>,
            lifecycle: Rc<RefCell<Vec<String>>>,
        }

        impl FakeGuest {
            fn new(
                generation_id: u64,
                state: Vec<u8>,
                dispatched: Rc<RefCell<Vec<(u64, StableId128)>>>,
                lifecycle: Rc<RefCell<Vec<String>>>,
            ) -> Self {
                Self {
                    generation_id,
                    state,
                    dispatched,
                    lifecycle,
                }
            }

            fn state(&self) -> &[u8] {
                &self.state
            }

            fn set_state(&mut self, state: Vec<u8>) {
                self.state = state;
            }

            fn dispatch(&self, callback_id: StableId128) {
                self.dispatched
                    .borrow_mut()
                    .push((self.generation_id, callback_id));
            }
        }

        impl ReloadGuest for FakeGuest {
            fn activate(&mut self) {
                self.lifecycle
                    .borrow_mut()
                    .push(format!("activated:{}", self.generation_id));
            }

            fn retire(&mut self) {
                self.lifecycle
                    .borrow_mut()
                    .push(format!("retired:{}", self.generation_id));
            }
        }

        struct TrackedResource {
            generation_id: u64,
            released: Rc<RefCell<Vec<String>>>,
        }

        impl TrackedResource {
            fn new(generation_id: u64, released: Rc<RefCell<Vec<String>>>) -> Self {
                Self {
                    generation_id,
                    released,
                }
            }
        }

        impl GenerationResource for TrackedResource {
            fn release(self: Box<Self>) {
                self.released
                    .borrow_mut()
                    .push(format!("released:{}", self.generation_id));
            }
        }

        struct CounterRoot {
            counter: Cell<u32>,
            child: Option<AnyElement>,
        }

        impl CounterRoot {
            fn new(counter: u32, child: Option<AnyElement>) -> AnyElement {
                Self {
                    counter: Cell::new(counter),
                    child,
                }
                .boxed()
            }

            fn of(root: &AnyElement) -> &Self {
                root.option_any()
                    .and_then(|value| value.downcast_ref::<Self>())
                    .unwrap()
            }
        }

        impl VisitorElement for CounterRoot {
            fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
                if let Some(child) = &self.child {
                    visitor(child.as_ref());
                }
            }

            fn debug_name(&self) -> &'static str {
                "CounterRoot"
            }
        }

        impl EventElement for CounterRoot {}
        impl LayoutElement for CounterRoot {}
        impl Drawable for CounterRoot {
            fn update(&self, _ctx: &BuildContext) {}
        }
        impl Rebuildable for CounterRoot {
            fn option_any(&self) -> Option<&dyn Any> {
                Some(self)
            }

            fn adopt_runtime_state_from(&self, old: &dyn Element) {
                let old = old
                    .option_any()
                    .and_then(|value| value.downcast_ref::<Self>())
                    .unwrap();
                self.counter.set(old.counter.get());
            }
        }

        struct GuestModule {
            widget_image: Vec<u8>,
            default_state: Vec<u8>,
        }

        impl GuestModule {
            fn encode(&self) -> Vec<u8> {
                let mut bytes =
                    Vec::with_capacity(8 + self.widget_image.len() + self.default_state.len());
                bytes.extend_from_slice(b"AGMD");
                bytes.extend_from_slice(&(self.widget_image.len() as u32).to_le_bytes());
                bytes.extend_from_slice(&self.widget_image);
                bytes.extend_from_slice(&self.default_state);
                bytes
            }

            fn decode(bytes: &[u8]) -> Option<Self> {
                if bytes.get(..4)? != b"AGMD" {
                    return None;
                }
                let image_len = u32::from_le_bytes(bytes.get(4..8)?.try_into().ok()?) as usize;
                let widget_image = bytes.get(8..8 + image_len)?.to_vec();
                Some(Self {
                    widget_image,
                    default_state: bytes.get(8 + image_len..)?.to_vec(),
                })
            }
        }

        fn guest_module(label: &str, callback_id: StableId128, default_state: Vec<u8>) -> Vec<u8> {
            GuestModule {
                widget_image: widget_image(label, callback_id),
                default_state,
            }
            .encode()
        }

        fn uncompilable_module() -> Vec<u8> {
            let nodes = [WidgetNode::new(WIDGET_TEXT, WIDGET_SCHEMA)];
            GuestModule {
                widget_image: WidgetDocument::new(0, 1, 0, &nodes, &[], &[])
                    .encode(MODEL_LIMITS)
                    .unwrap(),
                default_state: counter_state(9, STATE_V1, &[0]),
            }
            .encode()
        }

        fn widget_image(label: &str, callback_id: StableId128) -> Vec<u8> {
            let root_children = [1_u32, 2];
            let button_children = [3_u32];
            let label_properties = [WidgetProperty::new(
                PROPERTY_TEXT_CONTENT,
                PropertyValue::StringRef(0),
            )];
            let button_properties = [WidgetProperty::new(
                PROPERTY_TEXT_CONTENT,
                PropertyValue::StringRef(1),
            )];
            let callbacks = [CallbackBinding::new(
                EVENT_BUTTON_PRESS,
                WIDGET_SCHEMA,
                callback_id,
            )];
            let nodes = [
                WidgetNode::new(WIDGET_COLUMN, WIDGET_SCHEMA).children(&root_children),
                WidgetNode::new(WIDGET_TEXT, WIDGET_SCHEMA).properties(&label_properties),
                WidgetNode::new(WIDGET_BUTTON, WIDGET_SCHEMA)
                    .key(BUTTON_KEY)
                    .callbacks(&callbacks)
                    .children(&button_children),
                WidgetNode::new(WIDGET_TEXT, WIDGET_SCHEMA).properties(&button_properties),
            ];
            let strings = [label, "press"];
            WidgetDocument::new(0, 1, 0, &nodes, &strings, &[])
                .encode(MODEL_LIMITS)
                .unwrap()
        }

        fn bootstrap_state() -> Vec<u8> {
            StateBundle::new(APPLICATION_ID, BOOTSTRAP_GENERATION, &[])
                .encode(MODEL_LIMITS)
                .unwrap()
        }

        fn counter_state(source_generation: u64, version: Version, counter: &[u8]) -> Vec<u8> {
            let entries = [StateEntry::new(
                COUNTER_STATE,
                COUNTER_SCHEMA,
                version,
                StatePolicy::Required,
                counter,
            )];
            StateBundle::new(APPLICATION_ID, source_generation, &entries)
                .encode(MODEL_LIMITS)
                .unwrap()
        }

        fn counter_and_draft_state(
            source_generation: u64,
            counter_version: Version,
            counter: &[u8],
            draft_version: Version,
            draft: &[u8],
        ) -> Vec<u8> {
            let entries = [
                StateEntry::new(
                    COUNTER_STATE,
                    COUNTER_SCHEMA,
                    counter_version,
                    StatePolicy::Required,
                    counter,
                ),
                StateEntry::new(
                    DRAFT_STATE,
                    DRAFT_SCHEMA,
                    draft_version,
                    StatePolicy::ResetSafe,
                    draft,
                ),
            ];
            StateBundle::new(APPLICATION_ID, source_generation, &entries)
                .encode(MODEL_LIMITS)
                .unwrap()
        }

        fn widen_counter(payload: &[u8]) -> Result<Vec<u8>, StateMigrationFailure> {
            match payload {
                [counter] => Ok(vec![*counter, 0]),
                _ => Err(StateMigrationFailure::new(
                    "counter payload is not one byte",
                )),
            }
        }

        fn credentials() -> SessionCredentials {
            SessionCredentials::from_parts([0x11; 16], [0xA5; 32])
        }

        fn protocol_limits() -> ProtocolLimits {
            ProtocolLimits::new(8_192, Duration::from_secs(10))
                .max_chunk_bytes(64)
                .max_terminal_results(4)
        }

        fn listener_security() -> ListenerSecurity {
            ListenerSecurity::new(Duration::from_secs(60), 4, Duration::from_secs(1))
        }

        fn module_metadata() -> ModuleMetadata {
            ModuleMetadata::new([0x0A; 16], [0x0B; 16], 1, 0, [0x0C; 32])
        }

        fn spawn_command_listener(
            active_generation: u64,
            connections: usize,
        ) -> (
            SocketAddr,
            ProtocolReloadInbox,
            JoinHandle<Vec<ReloadConnectionOutcome>>,
        ) {
            let (sink, inbox) = reload_command_bridge(1, active_generation);
            let listener = ReloadCommandListener::bind_secure(
                "127.0.0.1:0",
                credentials(),
                protocol_limits(),
                listener_security(),
                sink,
            )
            .unwrap();
            let address = listener.local_addr().unwrap();
            let server = thread::spawn(move || {
                (0..connections)
                    .map(|_| listener.accept_connection().unwrap())
                    .collect()
            });
            (address, inbox, server)
        }

        fn spawn_command_client(
            address: SocketAddr,
            request_id: u64,
            module: Vec<u8>,
        ) -> JoinHandle<ReloadResult> {
            thread::spawn(move || {
                let mut stream = connect_loopback_client(address);
                send_reload_command(
                    &mut stream,
                    &credentials(),
                    protocol_limits(),
                    request_id,
                    module_metadata(),
                    &module,
                )
                .unwrap()
            })
        }

        fn connect_loopback_client(address: SocketAddr) -> TcpStream {
            let stream = TcpStream::connect_timeout(&address, TIMEOUT).unwrap();
            stream.set_read_timeout(Some(TIMEOUT)).unwrap();
            stream.set_write_timeout(Some(TIMEOUT)).unwrap();
            stream
        }

        fn command_results(server: JoinHandle<Vec<ReloadConnectionOutcome>>) -> Vec<ReloadResult> {
            server
                .join()
                .unwrap()
                .into_iter()
                .map(|outcome| match outcome {
                    ReloadConnectionOutcome::Command(result) => result,
                    ReloadConnectionOutcome::Query(result) => {
                        panic!("expected a command outcome, got a query result {result:?}")
                    }
                })
                .collect()
        }

        #[cfg(not(target_arch = "wasm32"))]
        fn dummy_async_handle() -> tokio::runtime::Handle {
            use std::sync::OnceLock;

            static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
            let runtime = RUNTIME.get_or_init(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
            });
            let _guard = runtime.enter();
            tokio::runtime::Handle::current()
        }

        fn context() -> BuildContext<'static> {
            let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
            BuildContext::new(
                aimer_canvas::FrameCanvas::new(inner),
                Default::default(),
                1.0,
                Default::default(),
                Default::default(),
                WindowHandle::headless(Default::default(), 1.0),
                #[cfg(not(target_arch = "wasm32"))]
                dummy_async_handle(),
            )
        }
    }
    #[cfg(feature = "wasm-hot-reload")]
    mod reload_listener_bridge {
        use std::net::TcpStream;
        use std::sync::mpsc;
        use std::thread;
        use std::time::Duration;

        use crate::hot_reload::{protocol_reload_stage, reload_command_bridge};
        use crate::reload_protocol::{
            ModuleMetadata, ProtocolLimits, ReloadResult, ReloadStage, SessionCredentials,
            send_reload_command,
        };
        use crate::reload_server::{ListenerSecurity, ReloadCommandListener};
        use aimer_anteros::ReloadStage as RuntimeReloadStage;

        #[test]
        fn authenticated_listener_waits_for_the_host_safe_point_result() {
            let credentials = SessionCredentials::from_parts([0x11; 16], [0xA5; 32]);
            let limits = ProtocolLimits::new(1024, Duration::from_secs(1)).max_chunk_bytes(3);
            let (sink, inbox) = reload_command_bridge(1, 1);
            let listener = ReloadCommandListener::bind_secure(
                "127.0.0.1:0",
                credentials.clone(),
                limits,
                ListenerSecurity::new(Duration::from_secs(60), 4, Duration::from_secs(1)),
                sink,
            )
            .unwrap();
            let address = listener.local_addr().unwrap();
            let server = thread::spawn(move || listener.accept_once().unwrap());
            let (client_tx, client_rx) = mpsc::sync_channel(1);
            let client = thread::spawn(move || {
                let mut stream = TcpStream::connect(address).unwrap();
                client_tx
                    .send(
                        send_reload_command(
                            &mut stream,
                            &credentials,
                            limits,
                            41,
                            ModuleMetadata::new([1; 16], [2; 16], 1, 0, [3; 32]),
                            b"\0asm\x01\0\0\0",
                        )
                        .unwrap(),
                    )
                    .unwrap();
            });

            let pending = inbox.recv_timeout(Duration::from_secs(1)).unwrap();
            assert_eq!(pending.command().request_id(), 41);
            assert!(matches!(
                client_rx.try_recv(),
                Err(mpsc::TryRecvError::Empty)
            ));
            let committed = ReloadResult::Committed {
                active_generation: 2,
                reset_state_entries: 0,
                cleanup_warnings: 0,
            };
            pending.complete(committed.clone()).unwrap();

            assert_eq!(
                client_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
                committed
            );
            assert_eq!(server.join().unwrap(), committed);
            client.join().unwrap();
        }

        #[test]
        fn every_transaction_boundary_has_one_stable_protocol_stage() {
            let cases = [
                (RuntimeReloadStage::Preflight, ReloadStage::Preflight),
                (RuntimeReloadStage::Instantiate, ReloadStage::Instantiate),
                (RuntimeReloadStage::Initialize, ReloadStage::Initialize),
                (RuntimeReloadStage::ExportState, ReloadStage::StateExport),
                (RuntimeReloadStage::MigrateState, ReloadStage::Migration),
                (RuntimeReloadStage::ImportState, ReloadStage::StateImport),
                (RuntimeReloadStage::Build, ReloadStage::Build),
                (RuntimeReloadStage::Validate, ReloadStage::Validation),
                (
                    RuntimeReloadStage::Materialize,
                    ReloadStage::Materialization,
                ),
                (
                    RuntimeReloadStage::PrepareReconciliation,
                    ReloadStage::Reconciliation,
                ),
                (
                    RuntimeReloadStage::PreCommitCancellation,
                    ReloadStage::Cancellation,
                ),
            ];

            for (runtime, protocol) in cases {
                assert_eq!(protocol_reload_stage(runtime), protocol);
            }
        }

        #[test]
        fn dropping_a_pending_command_reports_cancellation_without_changing_generation() {
            let credentials = SessionCredentials::from_parts([0x11; 16], [0xA5; 32]);
            let limits = ProtocolLimits::new(1024, Duration::from_secs(1));
            let (sink, inbox) = reload_command_bridge(1, 7);
            let listener = ReloadCommandListener::bind_secure(
                "127.0.0.1:0",
                credentials.clone(),
                limits,
                ListenerSecurity::new(Duration::from_secs(60), 4, Duration::from_secs(1)),
                sink,
            )
            .unwrap();
            let address = listener.local_addr().unwrap();
            let server = thread::spawn(move || listener.accept_once().unwrap());
            let client = thread::spawn(move || {
                let mut stream = TcpStream::connect(address).unwrap();
                send_reload_command(
                    &mut stream,
                    &credentials,
                    limits,
                    42,
                    ModuleMetadata::new([1; 16], [2; 16], 1, 0, [3; 32]),
                    b"candidate",
                )
                .unwrap()
            });

            drop(inbox.recv_timeout(Duration::from_secs(1)).unwrap());
            let result = client.join().unwrap();

            assert!(matches!(
                &result,
                ReloadResult::Rejected {
                    stage: ReloadStage::Cancellation,
                    active_generation: 7,
                    ..
                }
            ));
            assert_eq!(server.join().unwrap(), result);
            assert_eq!(inbox.active_generation(), 7);
        }

        #[test]
        fn slow_safe_point_never_becomes_a_false_terminal_rejection() {
            let credentials = SessionCredentials::from_parts([0x11; 16], [0xA5; 32]);
            let limits = ProtocolLimits::new(1024, Duration::from_secs(1));
            let (sink, inbox) = reload_command_bridge(1, 1);
            let listener = ReloadCommandListener::bind(
                "127.0.0.1:0",
                credentials.clone(),
                limits,
                ListenerSecurity::new(Duration::from_secs(60), 4, Duration::from_secs(1)),
                sink,
            )
            .unwrap();
            let address = listener.local_addr().unwrap();
            let server = thread::spawn(move || listener.accept_once().unwrap());
            let client = thread::spawn(move || {
                let mut stream = TcpStream::connect(address).unwrap();
                send_reload_command(
                    &mut stream,
                    &credentials,
                    limits,
                    43,
                    ModuleMetadata::new([1; 16], [2; 16], 1, 0, [3; 32]),
                    b"slow candidate",
                )
                .unwrap()
            });
            let pending = inbox.recv_timeout(Duration::from_secs(1)).unwrap();
            thread::sleep(Duration::from_millis(10));
            let committed = ReloadResult::Committed {
                active_generation: 2,
                reset_state_entries: 0,
                cleanup_warnings: 0,
            };

            pending.complete(committed.clone()).unwrap();

            assert_eq!(client.join().unwrap(), committed);
            assert_eq!(server.join().unwrap(), committed);
        }
    }
    #[cfg(feature = "wasm-hot-reload")]
    mod widget_materialization {
        use std::cell::Cell;
        use std::rc::Rc;

        use crate::hot_reload::{
            EVENT_BUTTON_PRESS, PROPERTY_CONTAINER_COLOR, PROPERTY_CONTAINER_WIDTH,
            PROPERTY_SIZED_BOX_HEIGHT, PROPERTY_SIZED_BOX_WIDTH, PROPERTY_TEXT_CONTENT,
            WIDGET_BUTTON, WIDGET_COLUMN, WIDGET_CONTAINER, WIDGET_ROW, WIDGET_SIZED_BOX,
            WIDGET_TEXT, materialize_aimer_widget_tree,
        };
        #[cfg(feature = "portable-guest")]
        use crate::hot_reload::{
            PROPERTY_CONTAINER_BOX_DECORATION, PROPERTY_CONTAINER_HEIGHT,
            PROPERTY_CONTAINER_MARGIN, PROPERTY_CONTAINER_PADDING,
        };
        use aimer_anteros::{
            BUILTIN_PORTABLE_WIDGET_SCHEMAS, CallbackBinding, EventId, ModelError, ModelLimits,
            PropertyId, PropertyValue, StableId128 as WireStableId128, Version, WidgetDocument,
            WidgetMaterializeError, WidgetNode, WidgetProperty, WidgetSchemaId,
        };
        use aimer_container::{Container, SizedBox};
        use aimer_flex::{Column, Row};
        use aimer_input::button::Button;
        #[cfg(feature = "portable-guest")]
        use aimer_style::{BoxDecoration, LayoutSpacing, Spacing};
        use aimer_text::Text;
        #[cfg(feature = "portable-guest")]
        use aimer_widget::PortableWidget;
        #[cfg(feature = "portable-guest")]
        use aimer_widget::base::Color;
        use aimer_widget::base::{BuildContext, WindowHandle};
        use aimer_widget::portable::{
            PORTABLE_NATIVE_WIDGET_SCHEMAS, PortableCallbackBinding, PortableWidgetSchema,
            linked_portable_native_widget_schemas,
        };
        #[cfg(feature = "portable-guest")]
        use aimer_widget::portable::{
            PortableAsyncLimits, PortableBuildContext, PortableCallbackError,
            PortableCallbackStart, PortableLimits, PortableWidgetLimits, SourceFingerprint,
            StableId128 as PortableStableId128,
        };
        use aimer_widget::{
            AnyElement, Element, ReconciliationMatchKind, RequiredChild, Widget,
            plan_element_reconciliation,
        };

        const LIMITS: ModelLimits = ModelLimits::new(8_192, 64, 256, 256).max_widget_depth(16);
        const CALLBACK_ID: WireStableId128 = WireStableId128::from_bytes([0x31; 16]);
        const BUTTON_KEY: WireStableId128 = WireStableId128::from_bytes([0x41; 16]);

        #[test]
        fn gesture_detector_materializes_its_child_and_callback_binding() {
            let gesture_type =
                WidgetSchemaId::from_canonical_name("aimer.widget:aimer_input::GestureDetector");
            let tap_event =
                EventId::from_canonical_name("aimer.event:aimer_input::GestureDetector:on_tap");
            let callbacks = [CallbackBinding::new(
                tap_event,
                Version::new(1, 0),
                CALLBACK_ID,
            )];
            let text_properties = [WidgetProperty::new(
                PROPERTY_TEXT_CONTENT,
                PropertyValue::StringRef(0),
            )];
            let text =
                WidgetNode::new(WIDGET_TEXT, Version::new(1, 0)).properties(&text_properties);
            let children = [0];
            let gesture = WidgetNode::new(gesture_type, Version::new(1, 0))
                .callbacks(&callbacks)
                .children(&children);
            let nodes = [text, gesture];
            let image = WidgetDocument::new(1, 1, 1, &nodes, &["back"], &[])
                .encode(LIMITS)
                .unwrap();

            let root = materialize_aimer_widget_tree(&image, LIMITS, &context(), |_| {});

            let root = match root {
                Ok(root) => root,
                Err(error) => panic!("GestureDetector materialization failed: {error}"),
            };
            assert_eq!(root.debug_name(), "GestureDetector");
        }

        #[test]
        fn mouse_region_materializes_its_child_and_callback_bindings() {
            let region_type =
                WidgetSchemaId::from_canonical_name("aimer.widget:aimer_input::MouseRegion");
            let enter_event =
                EventId::from_canonical_name("aimer.event:aimer_input::MouseRegion:on_hover_enter");
            let exit_event =
                EventId::from_canonical_name("aimer.event:aimer_input::MouseRegion:on_hover_exit");
            let callbacks = [
                CallbackBinding::new(enter_event, Version::new(1, 0), CALLBACK_ID),
                CallbackBinding::new(
                    exit_event,
                    Version::new(1, 0),
                    WireStableId128::from_bytes([0x32; 16]),
                ),
            ];
            let text_properties = [WidgetProperty::new(
                PROPERTY_TEXT_CONTENT,
                PropertyValue::StringRef(0),
            )];
            let text =
                WidgetNode::new(WIDGET_TEXT, Version::new(1, 0)).properties(&text_properties);
            let children = [0];
            let region = WidgetNode::new(region_type, Version::new(1, 0))
                .callbacks(&callbacks)
                .children(&children);
            let nodes = [text, region];
            let image = WidgetDocument::new(1, 1, 1, &nodes, &["back"], &[])
                .encode(LIMITS)
                .unwrap();

            let root = materialize_aimer_widget_tree(&image, LIMITS, &context(), |_| {});

            let root = match root {
                Ok(root) => root,
                Err(error) => panic!("MouseRegion materialization failed: {error}"),
            };
            assert_eq!(root.debug_name(), "MouseRegion");
        }

        #[derive(aimer_macro::PortableWidget)]
        #[portable_widget(id = "aimer_quiver.tests.DerivedLabel")]
        struct DerivedLabel {
            text: String,
        }

        impl DerivedLabel {
            #[inline]
            fn new() -> Self {
                Self {
                    text: String::new(),
                }
            }

            #[inline]
            fn text(mut self, text: String) -> Self {
                self.text = text;
                self
            }
        }

        impl Widget for DerivedLabel {
            fn to_element(self, ctx: &BuildContext) -> AnyElement {
                aimer_text::Text::new(self.text).to_element(ctx)
            }
        }

        #[derive(aimer_macro::PortableWidget)]
        #[portable_widget(id = "aimer_quiver.tests.DerivedFrame")]
        struct DerivedFrame<W = RequiredChild> {
            label: Option<String>,
            #[portable_child]
            child: W,
        }

        impl DerivedFrame {
            #[inline]
            fn new() -> Self {
                Self {
                    label: None,
                    child: RequiredChild,
                }
            }
        }

        impl<W> DerivedFrame<W> {
            #[inline]
            fn label(mut self, label: String) -> Self {
                self.label = Some(label);
                self
            }

            #[inline]
            fn child<N>(self, child: N) -> DerivedFrame<N> {
                DerivedFrame {
                    label: self.label,
                    child,
                }
            }
        }

        impl<W> Widget for DerivedFrame<W>
        where
            W: Widget,
        {
            fn to_element(self, ctx: &BuildContext) -> AnyElement {
                self.child.to_element(ctx)
            }
        }

        #[test]
        fn derive_registered_widget_materializes_without_a_quiver_registration() {
            let widget_type =
                WidgetSchemaId::from_canonical_name("aimer.widget:aimer_quiver.tests.DerivedLabel");
            let text_property = PropertyId::from_canonical_name(
                "aimer.property:aimer_quiver.tests.DerivedLabel:text",
            );
            let properties = [WidgetProperty::new(
                text_property,
                PropertyValue::StringRef(0),
            )];
            let nodes = [WidgetNode::new(widget_type, Version::new(1, 0)).properties(&properties)];
            let image = WidgetDocument::new(0, 1, 0, &nodes, &["Derived"], &[])
                .encode(LIMITS)
                .unwrap();

            assert!(
                linked_portable_native_widget_schemas()
                    .iter()
                    .any(|schema| schema.widget().id() == widget_type),
                "linked schemas: {:?}",
                PORTABLE_NATIVE_WIDGET_SCHEMAS
                    .iter()
                    .map(|schema| schema.widget().id())
                    .collect::<Vec<_>>(),
            );

            let root = materialize_aimer_widget_tree(&image, LIMITS, &context(), |_| {}).unwrap();

            assert_eq!(root.debug_name(), "RawTextWidget");
        }

        #[test]
        fn derive_registered_parent_adapts_materialized_children_and_keeps_optional_defaults() {
            let frame_type =
                WidgetSchemaId::from_canonical_name("aimer.widget:aimer_quiver.tests.DerivedFrame");
            let text_properties = [WidgetProperty::new(
                PROPERTY_TEXT_CONTENT,
                PropertyValue::StringRef(0),
            )];
            let text =
                WidgetNode::new(WIDGET_TEXT, Version::new(1, 0)).properties(&text_properties);
            let children = [0];
            let frame = WidgetNode::new(frame_type, Version::new(1, 0)).children(&children);
            let image = WidgetDocument::new(1, 1, 1, &[text, frame], &["Child"], &[])
                .encode(LIMITS)
                .unwrap();

            let root = materialize_aimer_widget_tree(&image, LIMITS, &context(), |_| {}).unwrap();

            assert_eq!(root.debug_name(), "RawTextWidget");
        }

        #[test]
        fn built_in_derived_schemas_match_the_permanent_host_contract() {
            assert_eq!(
                <Column<Text> as PortableWidgetSchema>::SCHEMA,
                built_in_schema(WIDGET_COLUMN),
            );
            assert_eq!(
                <Row<Text> as PortableWidgetSchema>::SCHEMA,
                built_in_schema(WIDGET_ROW),
            );
            assert_eq!(
                <Container<Text> as PortableWidgetSchema>::SCHEMA,
                built_in_schema(WIDGET_CONTAINER),
            );
            assert_eq!(
                <SizedBox<Text> as PortableWidgetSchema>::SCHEMA,
                built_in_schema(WIDGET_SIZED_BOX),
            );
            assert_eq!(
                <Text as PortableWidgetSchema>::SCHEMA,
                built_in_schema(WIDGET_TEXT),
            );
            assert_eq!(
                <Button<Text> as PortableWidgetSchema>::SCHEMA,
                built_in_schema(WIDGET_BUTTON),
            );
        }

        #[test]
        fn canonical_widget_ir_materializes_the_concrete_aimer_widget_set() {
            let column_children = [1, 2, 3];
            let row_children = [4, 5];
            let container_child = [6];
            let button_child = [7];
            let title_properties = [WidgetProperty::new(
                PROPERTY_TEXT_CONTENT,
                PropertyValue::StringRef(0),
            )];
            let sized_box_properties = [
                WidgetProperty::new(PROPERTY_SIZED_BOX_WIDTH, PropertyValue::BlobRef(0)),
                WidgetProperty::new(PROPERTY_SIZED_BOX_HEIGHT, PropertyValue::BlobRef(1)),
            ];
            let container_properties = [WidgetProperty::new(
                PROPERTY_CONTAINER_COLOR,
                PropertyValue::Rgba(0x112233FF),
            )];
            let card_properties = [WidgetProperty::new(
                PROPERTY_TEXT_CONTENT,
                PropertyValue::StringRef(1),
            )];
            let button_properties = [WidgetProperty::new(
                PROPERTY_TEXT_CONTENT,
                PropertyValue::StringRef(2),
            )];
            let callbacks = [CallbackBinding::new(
                EVENT_BUTTON_PRESS,
                Version::new(1, 0),
                CALLBACK_ID,
            )];
            let nodes = [
                WidgetNode::new(WIDGET_COLUMN, Version::new(1, 0)).children(&column_children),
                WidgetNode::new(WIDGET_TEXT, Version::new(1, 0)).properties(&title_properties),
                WidgetNode::new(WIDGET_ROW, Version::new(1, 0)).children(&row_children),
                WidgetNode::new(WIDGET_SIZED_BOX, Version::new(1, 0))
                    .properties(&sized_box_properties),
                WidgetNode::new(WIDGET_CONTAINER, Version::new(1, 0))
                    .properties(&container_properties)
                    .children(&container_child),
                WidgetNode::new(WIDGET_BUTTON, Version::new(1, 0))
                    .key(BUTTON_KEY)
                    .callbacks(&callbacks)
                    .children(&button_child),
                WidgetNode::new(WIDGET_TEXT, Version::new(1, 0)).properties(&card_properties),
                WidgetNode::new(WIDGET_TEXT, Version::new(1, 0)).properties(&button_properties),
            ];
            let strings = ["Phase 3", "Card", "Reload"];
            let width = [1_u8, 1, 0, 0, 64, 65];
            let height = [1_u8, 1, 0, 0, 0, 65];
            let image = WidgetDocument::new(7, 1, 0, &nodes, &strings, &[&width, &height])
                .encode(LIMITS)
                .unwrap();

            let root = materialize_aimer_widget_tree(&image, LIMITS, &context(), |_| {}).unwrap();

            let mut names = Vec::new();
            collect_names(root.as_ref(), &mut names);
            assert_eq!(root.debug_name(), "Column");
            for expected in [
                "Column",
                "RawTextWidget",
                "Row",
                "SizedBox",
                "Container",
                "Button",
            ] {
                assert!(names.contains(&expected), "missing {expected} in {names:?}");
            }
            assert!(
                find_by_name(root.as_ref(), "Button")
                    .unwrap()
                    .reconciliation_key()
                    .is_some()
            );
        }

        fn built_in_schema(
            widget_type: WidgetSchemaId,
        ) -> aimer_anteros::PortableWidgetSchemaMetadata<'static> {
            BUILTIN_PORTABLE_WIDGET_SCHEMAS
                .into_iter()
                .find(|schema| schema.widget().id() == widget_type)
                .expect("permanent host schema must exist")
        }

        #[cfg(feature = "portable-guest")]
        #[test]
        fn button_async_callback_completes_with_bounded_identity_and_retirement_rejection() {
            let completed = Rc::new(Cell::new(0_u8));
            let completed_by_callback = Rc::clone(&completed);
            let source = SourceFingerprint::new(PortableStableId128::from_bytes([0x71; 16]));
            let mut build =
                portable_context().with_async_limits(PortableAsyncLimits::new(1, 4, 8, 1));
            let button = Button::new().on_press_async(move || {
                let completed = Rc::clone(&completed_by_callback);
                async move {
                    completed.set(completed.get().saturating_add(1));
                }
            });
            let child = Text::new("async")
                .to_portable_node(&mut build, source.child(0))
                .unwrap();
            let callback_metadata =
                <Button<RequiredChild> as PortableWidgetSchema>::SCHEMA.callbacks()[0];
            assert_eq!(callback_metadata.id(), EVENT_BUTTON_PRESS);
            let callback = PortableCallbackBinding::bind_portable_callback(
                button.on_press,
                &build,
                None,
                source,
                callback_metadata,
                "Button",
            )
            .unwrap()
            .unwrap();
            let root = build
                .push_node_with_callbacks(
                    WIDGET_BUTTON,
                    Version::new(1, 0),
                    None,
                    source,
                    &[],
                    vec![callback],
                    &[child],
                )
                .unwrap();
            let document = build.finish_document(root).unwrap();
            let encoded = document.encode().unwrap();
            let view = aimer_anteros::WidgetDocumentView::decode(&encoded, LIMITS).unwrap();
            let binding = view.node(root.index()).unwrap().callbacks().next().unwrap();

            assert_eq!(view.generation_id(), 7);
            assert_eq!(binding.event_kind(), EVENT_BUTTON_PRESS);
            assert_eq!(binding.async_schema(), Some(Version::new(1, 0)));
            let native_root =
                materialize_aimer_widget_tree(&encoded, LIMITS, &context(), |_| {}).unwrap();
            assert_eq!(native_root.debug_name(), "Button");

            let callback_id = PortableStableId128::from_bytes(*binding.callback_id().as_bytes());
            let registry = build.callback_registry();
            let task_id = match registry.dispatch_start(callback_id, &mut build).unwrap() {
                PortableCallbackStart::Started { task_id } => task_id,
                PortableCallbackStart::Completed => {
                    panic!("async Button callback completed synchronously")
                }
            };
            assert_eq!(task_id.value(), 1);
            assert_eq!(build.async_task_count(), 1);

            build.run_async_microtasks();

            assert_eq!(completed.get(), 1);
            assert_eq!(build.async_task_count(), 0);
            assert!(build.take_rebuild_request());

            let next = Text::new(completed.get().to_string())
                .to_portable_node(
                    &mut build,
                    SourceFingerprint::new(PortableStableId128::from_bytes([0x72; 16])),
                )
                .unwrap();
            let next_document = build.finish_document(next).unwrap();
            let next_encoded = next_document.encode().unwrap();
            let next_view =
                aimer_anteros::WidgetDocumentView::decode(&next_encoded, LIMITS).unwrap();
            let next_node = next_view.node(next.index()).unwrap();
            let text_index = match next_node.properties().next().unwrap().value() {
                PropertyValue::StringRef(index) => index,
                value => panic!("updated text used unexpected property value: {value:?}"),
            };
            assert_eq!(next_view.string(text_index), Some("1"));
            assert_eq!(
                materialize_aimer_widget_tree(&next_encoded, LIMITS, &context(), |_| {})
                    .unwrap()
                    .debug_name(),
                "RawTextWidget"
            );
            assert_eq!(
                registry.dispatch_start(callback_id, &mut build),
                Err(PortableCallbackError::Retired)
            );
        }

        #[cfg(feature = "portable-guest")]
        #[test]
        fn button_async_callback_failure_keeps_task_identity_and_bounded_diagnostic() {
            let source = SourceFingerprint::new(PortableStableId128::from_bytes([0x73; 16]));
            let mut build =
                portable_context().with_async_limits(PortableAsyncLimits::new(1, 4, 0, 1));
            let button = Button::new().on_press_async(|| async {});
            let child = Text::new("failure")
                .to_portable_node(&mut build, source.child(0))
                .unwrap();
            let callback_metadata =
                <Button<RequiredChild> as PortableWidgetSchema>::SCHEMA.callbacks()[0];
            assert_eq!(callback_metadata.id(), EVENT_BUTTON_PRESS);
            let callback = PortableCallbackBinding::bind_portable_callback(
                button.on_press,
                &build,
                None,
                source,
                callback_metadata,
                "Button",
            )
            .unwrap()
            .unwrap();
            let root = build
                .push_node_with_callbacks(
                    WIDGET_BUTTON,
                    Version::new(1, 0),
                    None,
                    source,
                    &[],
                    vec![callback],
                    &[child],
                )
                .unwrap();
            build.finish_document(root).unwrap();
            let callback_id = build.callback_id_for(None, source, EVENT_BUTTON_PRESS);
            let registry = build.callback_registry();
            let task_id = match registry.dispatch_start(callback_id, &mut build).unwrap() {
                PortableCallbackStart::Started { task_id } => task_id,
                PortableCallbackStart::Completed => {
                    panic!("async Button callback completed synchronously")
                }
            };

            build.run_async_microtasks();

            let failure = build.take_async_failure().expect("bounded async failure");
            assert_eq!(failure.task_id(), task_id);
            assert_eq!(failure.callback_id(), callback_id);
            assert_eq!(failure.message(), "async callback fuel exhausted");
            assert_eq!(build.async_task_count(), 0);
            assert!(build.take_rebuild_request());
        }

        #[cfg(feature = "portable-guest")]
        #[test]
        fn derived_container_round_trip_materializes_every_portable_field() {
            let mut build = portable_context();
            let root = Container::new()
                .width(320.0)
                .height(180.0)
                .padding(LayoutSpacing::all(Spacing::Px(20)))
                .margin(LayoutSpacing::all(Spacing::Px(4)))
                .box_decoration(BoxDecoration::new().background_color(Color::GREEN))
                .color(Color::HexA(0x112233FF))
                .child(Text::new("all fields"))
                .to_portable_node(
                    &mut build,
                    SourceFingerprint::new(PortableStableId128::from_path(
                        "test",
                        "container-all-fields",
                    )),
                )
                .unwrap();
            let document = build.finish_document(root).unwrap();
            let encoded = document.encode().unwrap();
            let view = aimer_anteros::WidgetDocumentView::decode(&encoded, LIMITS).unwrap();
            let container = view.node(root.index()).unwrap();

            assert_eq!(container.widget_type(), WIDGET_CONTAINER);
            assert!(container.properties().any(|property| {
                property.property_id() == PROPERTY_CONTAINER_WIDTH
                    && matches!(property.value(), PropertyValue::BlobRef(index)
                        if view.blob(index) == Some(&[1, 1, 0, 0, 160, 67][..]))
            }));
            assert!(container.properties().any(|property| {
                property.property_id() == PROPERTY_CONTAINER_HEIGHT
                    && matches!(property.value(), PropertyValue::BlobRef(index)
                        if view.blob(index) == Some(&[1, 1, 0, 0, 52, 67][..]))
            }));
            for property_id in [
                PROPERTY_CONTAINER_PADDING,
                PROPERTY_CONTAINER_MARGIN,
                PROPERTY_CONTAINER_BOX_DECORATION,
            ] {
                assert!(container.properties().any(|property| {
                    property.property_id() == property_id
                        && matches!(property.value(), PropertyValue::BlobRef(_))
                }));
            }
            assert!(container.properties().any(|property| {
                property.property_id() == PROPERTY_CONTAINER_COLOR
                    && matches!(property.value(), PropertyValue::Rgba(0x112233FF))
            }));

            let root = materialize_aimer_widget_tree(&encoded, LIMITS, &context(), |_| {}).unwrap();
            let mut names = Vec::new();
            collect_names(root.as_ref(), &mut names);
            assert_eq!(root.debug_name(), "Container");
            assert!(
                names.contains(&"Container"),
                "missing Container in {names:?}"
            );
            assert!(
                names.contains(&"RawTextWidget"),
                "missing text child in {names:?}"
            );
        }

        #[test]
        fn concrete_widget_schema_rejects_required_field_type_child_and_callback_errors() {
            let cases = [
                (
                    encode_single(WidgetNode::new(WIDGET_TEXT, Version::new(1, 0)), &[]),
                    ModelError::MissingWidgetProperty {
                        node: 0,
                        widget_type: WIDGET_TEXT,
                        property_id: PROPERTY_TEXT_CONTENT,
                    },
                ),
                (
                    encode_single(
                        WidgetNode::new(WIDGET_TEXT, Version::new(1, 0)).properties(&[
                            WidgetProperty::new(PROPERTY_TEXT_CONTENT, PropertyValue::StringRef(0)),
                            WidgetProperty::new(PROPERTY_TEXT_CONTENT, PropertyValue::StringRef(0)),
                        ]),
                        &["duplicate"],
                    ),
                    ModelError::DuplicateWidgetProperty {
                        node: 0,
                        widget_type: WIDGET_TEXT,
                        property_id: PROPERTY_TEXT_CONTENT,
                    },
                ),
                (
                    encode_single(
                        WidgetNode::new(WIDGET_SIZED_BOX, Version::new(1, 0)).properties(&[
                            WidgetProperty::new(
                                PROPERTY_SIZED_BOX_WIDTH,
                                PropertyValue::Bool(true),
                            ),
                        ]),
                        &[],
                    ),
                    ModelError::InvalidWidgetPropertyType {
                        node: 0,
                        widget_type: WIDGET_SIZED_BOX,
                        property_id: PROPERTY_SIZED_BOX_WIDTH,
                    },
                ),
                (
                    {
                        let invalid_dimension = [1_u8, 1, 0, 0, 128, 191];
                        encode_single_with_blobs(
                            WidgetNode::new(WIDGET_SIZED_BOX, Version::new(1, 0)).properties(&[
                                WidgetProperty::new(
                                    PROPERTY_SIZED_BOX_HEIGHT,
                                    PropertyValue::BlobRef(0),
                                ),
                            ]),
                            &[],
                            &[&invalid_dimension],
                        )
                    },
                    ModelError::InvalidWidgetPropertyValue {
                        node: 0,
                        widget_type: WIDGET_SIZED_BOX,
                        property_id: PROPERTY_SIZED_BOX_HEIGHT,
                    },
                ),
                (
                    {
                        let children = [1];
                        let nodes = [
                            WidgetNode::new(WIDGET_SIZED_BOX, Version::new(1, 0))
                                .children(&children),
                            WidgetNode::new(WIDGET_SIZED_BOX, Version::new(1, 0)),
                        ];
                        WidgetDocument::new(1, 1, 0, &nodes, &[], &[])
                            .encode(LIMITS)
                            .unwrap()
                    },
                    ModelError::InvalidWidgetChildCount {
                        node: 0,
                        widget_type: WIDGET_SIZED_BOX,
                        count: 1,
                        minimum: 0,
                        maximum: 0,
                    },
                ),
                (
                    encode_single(
                        WidgetNode::new(WIDGET_SIZED_BOX, Version::new(1, 0)).properties(&[
                            WidgetProperty::new(PROPERTY_CONTAINER_WIDTH, PropertyValue::F64(12.0)),
                        ]),
                        &[],
                    ),
                    ModelError::UnsupportedWidgetProperty {
                        node: 0,
                        widget_type: WIDGET_SIZED_BOX,
                        property_id: PROPERTY_CONTAINER_WIDTH,
                    },
                ),
                (
                    encode_single(
                        WidgetNode::new(WIDGET_SIZED_BOX, Version::new(1, 0)).properties(&[
                            WidgetProperty::new(PropertyId::new(99), PropertyValue::Bool(true)),
                        ]),
                        &[],
                    ),
                    ModelError::UnsupportedWidgetProperty {
                        node: 0,
                        widget_type: WIDGET_SIZED_BOX,
                        property_id: PropertyId::new(99),
                    },
                ),
                (
                    {
                        let child = [1];
                        let callbacks = [
                            CallbackBinding::new(
                                EVENT_BUTTON_PRESS,
                                Version::new(1, 0),
                                CALLBACK_ID,
                            ),
                            CallbackBinding::new(
                                EVENT_BUTTON_PRESS,
                                Version::new(1, 0),
                                WireStableId128::from_bytes([0x32; 16]),
                            ),
                        ];
                        let text = [WidgetProperty::new(
                            PROPERTY_TEXT_CONTENT,
                            PropertyValue::StringRef(0),
                        )];
                        let nodes = [
                            WidgetNode::new(WIDGET_BUTTON, Version::new(1, 0))
                                .children(&child)
                                .callbacks(&callbacks),
                            WidgetNode::new(WIDGET_TEXT, Version::new(1, 0)).properties(&text),
                        ];
                        WidgetDocument::new(1, 1, 0, &nodes, &["label"], &[])
                            .encode(LIMITS)
                            .unwrap()
                    },
                    ModelError::InvalidWidgetCallbackCount {
                        node: 0,
                        widget_type: WIDGET_BUTTON,
                        count: 2,
                        maximum: 1,
                    },
                ),
                (
                    {
                        let children = [1];
                        let nodes = [
                            WidgetNode::new(WIDGET_SIZED_BOX, Version::new(1, 0))
                                .children(&children),
                            WidgetNode::new(WIDGET_SIZED_BOX, Version::new(1, 0)),
                        ];
                        WidgetDocument::new(1, 1, 0, &nodes, &[], &[])
                            .encode(LIMITS)
                            .unwrap()
                    },
                    ModelError::InvalidWidgetChildCount {
                        node: 0,
                        widget_type: WIDGET_SIZED_BOX,
                        count: 1,
                        minimum: 0,
                        maximum: 0,
                    },
                ),
                (
                    encode_single(
                        WidgetNode::new(WIDGET_TEXT, Version::new(1, 0))
                            .properties(&[WidgetProperty::new(
                                PROPERTY_TEXT_CONTENT,
                                PropertyValue::StringRef(0),
                            )])
                            .callbacks(&[CallbackBinding::new(
                                EVENT_BUTTON_PRESS,
                                Version::new(1, 0),
                                CALLBACK_ID,
                            )]),
                        &["label"],
                    ),
                    ModelError::UnsupportedWidgetCallback {
                        node: 0,
                        widget_type: WIDGET_TEXT,
                        event_kind: EVENT_BUTTON_PRESS,
                    },
                ),
            ];

            for (image, expected) in cases {
                let error = match materialize_aimer_widget_tree(&image, LIMITS, &context(), |_| {})
                {
                    Ok(_) => panic!("invalid concrete widget schema unexpectedly materialized"),
                    Err(error) => error,
                };
                assert_eq!(error, WidgetMaterializeError::Model(expected));
            }
        }

        #[test]
        fn unknown_optional_property_is_ignored_for_forward_compatibility() {
            let image = encode_single(
                WidgetNode::new(WIDGET_SIZED_BOX, Version::new(1, 0)).properties(&[
                    WidgetProperty::new(PropertyId::new(99), PropertyValue::Bool(true)).optional(),
                ]),
                &[],
            );

            let root = materialize_aimer_widget_tree(&image, LIMITS, &context(), |_| {}).unwrap();

            assert_eq!(root.debug_name(), "SizedBox");
        }

        #[test]
        fn keyed_materialized_widgets_match_after_moves_but_not_across_widget_types() {
            let old = materialize_aimer_widget_tree(
                &keyed_pair_image(false, WIDGET_BUTTON),
                LIMITS,
                &context(),
                |_| {},
            )
            .unwrap();
            let moved = materialize_aimer_widget_tree(
                &keyed_pair_image(true, WIDGET_BUTTON),
                LIMITS,
                &context(),
                |_| {},
            )
            .unwrap();
            let incompatible = materialize_aimer_widget_tree(
                &keyed_pair_image(true, WIDGET_CONTAINER),
                LIMITS,
                &context(),
                |_| {},
            )
            .unwrap();

            let moved_plan = plan_element_reconciliation(old.as_ref(), moved.as_ref());
            moved_plan.validate().unwrap();
            assert_eq!(keyed_match_count(&moved_plan), 2);

            let incompatible_plan =
                plan_element_reconciliation(old.as_ref(), incompatible.as_ref());
            incompatible_plan.validate().unwrap();
            assert_eq!(keyed_match_count(&incompatible_plan), 1);
        }

        fn keyed_pair_image(reversed: bool, first_widget_type: WidgetSchemaId) -> Vec<u8> {
            let root_children = if reversed { [3, 1] } else { [1, 3] };
            let first_child = [2];
            let second_child = [4];
            let first_text = [WidgetProperty::new(
                PROPERTY_TEXT_CONTENT,
                PropertyValue::StringRef(0),
            )];
            let second_text = [WidgetProperty::new(
                PROPERTY_TEXT_CONTENT,
                PropertyValue::StringRef(1),
            )];
            let nodes = [
                WidgetNode::new(WIDGET_COLUMN, Version::new(1, 0)).children(&root_children),
                WidgetNode::new(first_widget_type, Version::new(1, 0))
                    .key(WireStableId128::from_bytes([0x11; 16]))
                    .children(&first_child),
                WidgetNode::new(WIDGET_TEXT, Version::new(1, 0)).properties(&first_text),
                WidgetNode::new(WIDGET_BUTTON, Version::new(1, 0))
                    .key(WireStableId128::from_bytes([0x22; 16]))
                    .children(&second_child),
                WidgetNode::new(WIDGET_TEXT, Version::new(1, 0)).properties(&second_text),
            ];
            WidgetDocument::new(1, 1, 0, &nodes, &["first", "second"], &[])
                .encode(LIMITS)
                .unwrap()
        }

        fn keyed_match_count(plan: &aimer_widget::ReconciliationPlan<'_>) -> usize {
            plan.matches()
                .iter()
                .filter(|element_match| element_match.kind() == ReconciliationMatchKind::Keyed)
                .count()
        }

        fn encode_single(node: WidgetNode<'_>, strings: &[&str]) -> Vec<u8> {
            WidgetDocument::new(1, 1, 0, &[node], strings, &[])
                .encode(LIMITS)
                .unwrap()
        }

        fn encode_single_with_blobs(
            node: WidgetNode<'_>,
            strings: &[&str],
            blobs: &[&[u8]],
        ) -> Vec<u8> {
            WidgetDocument::new(1, 1, 0, &[node], strings, blobs)
                .encode(LIMITS)
                .unwrap()
        }

        fn collect_names<'a>(element: &'a dyn Element, names: &mut Vec<&'a str>) {
            names.push(element.debug_name());
            element.visit_children(&mut |child| collect_names(child, names));
        }

        fn find_by_name<'a>(element: &'a dyn Element, name: &str) -> Option<&'a dyn Element> {
            if element.debug_name() == name {
                return Some(element);
            }
            let mut result = None;
            element.visit_children(&mut |child| {
                if result.is_none() {
                    result = find_by_name(child, name);
                }
            });
            result
        }

        #[cfg(not(target_arch = "wasm32"))]
        fn dummy_async_handle() -> tokio::runtime::Handle {
            use std::sync::OnceLock;

            static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
            let runtime = RUNTIME.get_or_init(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
            });
            let _guard = runtime.enter();
            tokio::runtime::Handle::current()
        }

        fn context() -> BuildContext<'static> {
            let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
            BuildContext::new(
                aimer_canvas::FrameCanvas::new(inner),
                Default::default(),
                1.0,
                Default::default(),
                Default::default(),
                WindowHandle::headless(Default::default(), 1.0),
                #[cfg(not(target_arch = "wasm32"))]
                dummy_async_handle(),
            )
        }

        #[cfg(feature = "portable-guest")]
        fn portable_context() -> PortableBuildContext {
            PortableBuildContext::new(
                7,
                11,
                PortableWidgetLimits::new(8, 8, 8, 8, 64, 2_048).with_max_blob_bytes(4_096),
                PortableLimits::new(8, 16, 64, 128, 1_024),
            )
            .unwrap()
        }
    }
}
