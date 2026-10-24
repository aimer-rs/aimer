//! The persistence layer on a real browser's IndexedDB.
//!
//! Run with `scripts/web_tests.sh`. The native suite covers the logic; this
//! covers what only a browser can: that `AppStorage` wires the right databases,
//! that data survives opening the app again, and that saving works with the
//! web's cooperative scheduling, where nothing may block.

#![cfg(target_arch = "wasm32")]

use std::cell::Cell;
use std::time::Duration;

use aimer_persist::{
    AppStorage, Document, FlushOutcome, Persisted, Preferences, StorageLocation,
    flush_all_blocking, has_unsaved_changes, impl_storage_value,
};
use aimer_venus::{FrameBudget, Lifecycle, Venus, notify_lifecycle};
use js_sys::{Function, Promise};
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

thread_local! {
    static NEXT: Cell<u32> = const { Cell::new(0) };
}

/// An app id no other test (or earlier run) has used.
fn app_id(test: &str) -> String {
    let n = NEXT.with(|next| {
        next.set(next.get() + 1);
        next.get()
    });
    format!("persist-{test}-{}-{n}", js_sys::Date::now() as u64)
}

/// Gives the browser's event loop a turn, which is what completes IndexedDB
/// requests.
async fn tick() {
    let promise = Promise::new(&mut |resolve: Function, _| {
        let schedule = Function::new_with_args("resolve", "setTimeout(resolve, 1)");
        schedule.call1(&wasm_bindgen::JsValue::NULL, &resolve).expect("setTimeout");
    });
    JsFuture::from(promise).await.expect("timer");
}

#[derive(Clone, Debug, PartialEq)]
struct Notes {
    titles: Vec<String>,
}
impl_storage_value!(Notes { titles });

// ─────────────────────────────── locations ───────────────────────────────

#[wasm_bindgen_test]
async fn locations_are_backed_by_the_right_stores() {
    let app = AppStorage::new(&app_id("locations")).expect("app storage");

    app.data().write("ns", "k", b"data").await.expect("data");
    app.cache().expect("cache").write("ns", "k", b"cache").await.expect("cache");
    app.temp().expect("temp").write("ns", "k", b"temp").await.expect("temp");

    assert_eq!(app.data().read("ns", "k").await.expect("read"), Some(b"data".to_vec()));
    assert_eq!(
        app.cache().expect("cache").read("ns", "k").await.expect("read"),
        Some(b"cache".to_vec())
    );
    assert_eq!(
        app.at(StorageLocation::Roaming).expect("roaming").read("ns", "k").await.expect("read"),
        Some(b"data".to_vec()),
        "there is no roaming store on the web, so it is the data store"
    );
}

#[wasm_bindgen_test]
async fn data_and_cache_persist_but_temp_is_only_memory() {
    let id = app_id("persistence");
    let first = AppStorage::new(&id).expect("app storage");
    first.data().write("ns", "k", b"data").await.expect("data");
    first.cache().expect("cache").write("ns", "k", b"cache").await.expect("cache");
    first.temp().expect("temp").write("ns", "k", b"temp").await.expect("temp");
    drop(first);

    let second = AppStorage::new(&id).expect("app storage again");
    assert_eq!(second.data().read("ns", "k").await.expect("read"), Some(b"data".to_vec()));
    assert_eq!(
        second.cache().expect("cache").read("ns", "k").await.expect("read"),
        Some(b"cache".to_vec())
    );
    assert_eq!(
        second.temp().expect("temp").read("ns", "k").await.expect("read"),
        None,
        "scratch space on the web does not outlive the app"
    );
}

#[wasm_bindgen_test]
fn an_invalid_app_id_is_rejected() {
    assert!(AppStorage::new("../escape").is_err());
    assert!(AppStorage::new("").is_err());
}

// ──────────────────────────── preferences and documents ────────────────────────────

#[wasm_bindgen_test]
async fn preferences_survive_opening_the_app_again() {
    let id = app_id("preferences");
    let app = AppStorage::new(&id).expect("app storage");
    let prefs = app.preferences().await.expect("load");
    assert!(prefs.is_empty());

    prefs.set("dark", &true);
    prefs.set("name", &"Ada".to_owned());
    prefs.set("volume", &0.25f32);
    prefs.remove("volume");
    prefs.flush().await.expect("flush");
    assert!(!prefs.has_unsaved_changes());
    drop((prefs, app));

    let reopened = AppStorage::new(&id).expect("app storage again");
    let prefs = reopened.preferences().await.expect("load again");
    assert_eq!(prefs.get::<bool>("dark"), Ok(Some(true)));
    assert_eq!(prefs.get::<String>("name"), Ok(Some("Ada".to_owned())));
    assert_eq!(prefs.get::<f32>("volume"), Ok(None));
}

#[wasm_bindgen_test]
async fn a_document_round_trips_and_migrates_in_the_browser() {
    let id = app_id("document");
    let app = AppStorage::new(&id).expect("app storage");

    let v1 = app.document::<Vec<String>>("notes");
    v1.save(&vec!["a".to_owned(), "b".to_owned()]).await.expect("save v1");

    let v2 = app
        .document::<Notes>("notes")
        .version(2)
        .migrate(1, 2, |old| Ok(old.to_vec()));
    assert_eq!(
        v2.load().await.expect("load"),
        Some(Notes { titles: vec!["a".to_owned(), "b".to_owned()] })
    );

    // And a newer app's data is refused, here as everywhere.
    let newer = Document::<u32>::new(app.data(), "future").version(9);
    newer.save(&1).await.expect("save");
    assert!(matches!(
        Document::<u32>::new(app.data(), "future").version(2).load().await,
        Err(aimer_persist::PersistError::NewerVersion { found: 9, supported: 2 })
    ));
}

#[wasm_bindgen_test]
async fn persisted_state_is_saved_and_restored() {
    let id = app_id("persisted");
    let app = AppStorage::new(&id).expect("app storage");
    let persisted = Persisted::load(app.document::<Notes>("notes"), || Notes { titles: vec![] })
        .await
        .expect("load");

    let handle = persisted.handle();
    handle.update(|notes| notes.titles.push("one".to_owned()));
    handle.update(|notes| notes.titles.push("two".to_owned()));
    persisted.flush().await.expect("flush");
    drop((handle, persisted, app));

    let app = AppStorage::new(&id).expect("app storage again");
    let restored = Persisted::load(app.document::<Notes>("notes"), || Notes { titles: vec![] })
        .await
        .expect("load again");
    assert_eq!(restored.handle().read().titles, ["one", "two"]);
}

// ──────────────────────── saving in the web's scheduling model ────────────────────────

/// A web app has a runtime installed, and the browser drives it frame by frame.
/// Saving must make progress through idle work and IndexedDB's callbacks alone.
#[wasm_bindgen_test]
async fn changes_are_saved_in_the_background_by_the_runtime() {
    let venus = Venus::new();
    venus.install();
    let id = app_id("background");
    let app = AppStorage::new(&id).expect("app storage");
    let prefs = app.preferences().await.expect("load");

    prefs.set("draft", &"typed".to_owned());
    assert!(prefs.has_unsaved_changes());

    // What the event loop does between browser tasks.
    for _ in 0..500 {
        if !prefs.has_unsaved_changes() {
            break;
        }
        venus.run_microtasks();
        venus.run_idle(&FrameBudget::from_now(Duration::from_millis(8)));
        tick().await;
    }
    assert!(!prefs.has_unsaved_changes(), "the background save never finished");
    assert_eq!(prefs.last_save_error(), None);
    drop((prefs, app));

    let reopened = AppStorage::new(&id).expect("app storage again");
    let prefs = reopened.preferences().await.expect("load again");
    assert_eq!(prefs.get::<String>("draft"), Ok(Some("typed".to_owned())));
    Venus::uninstall();
}

/// Nothing may block on the web. The lifecycle path has to start the save and
/// return, and say honestly that it could not wait.
#[wasm_bindgen_test]
async fn going_to_the_background_starts_saving_without_blocking() {
    let venus = Venus::new();
    venus.install();
    let id = app_id("lifecycle");
    let app = AppStorage::new(&id).expect("app storage");
    let prefs = app.preferences().await.expect("load");
    prefs.set("draft", &"unsaved".to_owned());

    // Would hang the tab if it tried to wait for IndexedDB.
    notify_lifecycle(Lifecycle::Background);
    assert_eq!(
        flush_all_blocking(Duration::from_secs(5)),
        FlushOutcome::TimedOut { unsaved: 1 },
        "the web cannot block, so it must report that it did not wait"
    );
    assert!(has_unsaved_changes());

    // The started save finishes by itself as the browser runs.
    for _ in 0..500 {
        if !prefs.has_unsaved_changes() {
            break;
        }
        venus.run_microtasks();
        venus.run_idle(&FrameBudget::from_now(Duration::from_millis(8)));
        tick().await;
    }
    assert!(!prefs.has_unsaved_changes());
    drop((prefs, app));

    let reopened = AppStorage::new(&id).expect("app storage again");
    let prefs = reopened.preferences().await.expect("load again");
    assert_eq!(prefs.get::<String>("draft"), Ok(Some("unsaved".to_owned())));
    Venus::uninstall();
}

#[wasm_bindgen_test]
async fn flush_works_without_a_runtime_by_saving_inline() {
    Venus::uninstall();
    let id = app_id("inline");
    let app = AppStorage::new(&id).expect("app storage");
    let prefs = Preferences::empty(app.data());
    prefs.set("a", &1u8);

    prefs.flush().await.expect("flush");

    assert!(!prefs.has_unsaved_changes());
    let reopened = AppStorage::new(&id).expect("app storage again");
    assert_eq!(
        reopened.preferences().await.expect("load").get::<u8>("a"),
        Ok(Some(1))
    );
}
