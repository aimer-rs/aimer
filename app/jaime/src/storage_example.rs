//! Jaime's deterministic durable-storage capability and fallback example.
//!
//! The default page uses the platform-neutral memory adapter. Native and web
//! adapters can later replace it without changing the widget or the public
//! storage contract.

use aimer::console::SyncFuture;
use aimer::storage::{MemoryStorage, Storage, StorageResult};
use aimer::{AimerApp, AnyElement, BuildContext, Column, Container, Text, Widget};

/// Performs a namespaced memory round trip for the visible example and tests.
pub fn memory_storage_round_trip() -> StorageResult<Option<Vec<u8>>> {
    let storage = MemoryStorage::new();
    storage
        .write("preferences", "display-mode", b"dark")
        .block()?;
    storage
        .read("preferences", "display-mode")
        .block()
}

/// Demonstrates a typed fallback for an invalid storage key.
pub fn memory_storage_invalid_key_fallback() -> StorageResult<Option<Vec<u8>>> {
    MemoryStorage::new()
        .read("preferences", "")
        .block()
}

/// Builds the storage capability/fallback page without starting an app.
pub fn storage_example() -> impl Widget {
    StorageExample
}

/// Starts the storage example as a standalone Jaime application.
pub fn start_storage_example() {
    AimerApp::start(storage_example());
}

struct StorageExample;

impl Widget for StorageExample {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let value = memory_storage_round_trip()
            .map(|bytes| {
                bytes
                    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                    .unwrap_or_else(|| "missing".to_owned())
            })
            .unwrap_or_else(|error| format!("fallback: {error:?}"));
        let fallback = memory_storage_invalid_key_fallback()
            .map(|_| "unexpected success".to_owned())
            .unwrap_or_else(|error| format!("{error:?}"));

        Container::new()
            .child(
                Column::new().children([
                    Text::new("Durable storage").boxed(),
                    Text::new(format!("Memory adapter · preferences/display-mode = {value}"))
                        .wrapped()
                        .boxed(),
                    Text::new(format!("Invalid-key fallback = {fallback}"))
                        .wrapped()
                        .boxed(),
                    Text::new(
                        "Unsupported native or web backends should report a typed fallback instead of silently losing data.",
                    )
                    .wrapped()
                    .boxed(),
                ]),
            )
            .to_element(ctx)
    }

    fn debug_name(&self) -> &'static str {
        "StorageExample"
    }
}

impl aimer::PortableWidget for StorageExample {}
