//! Linux: a `StatusNotifierItem` on the session bus.
//!
//! The desktop draws the tray (KDE Plasma natively, GNOME through the AppIndicator
//! extension, most other panels through a plugin); an application describes its
//! icon and menu over D-Bus and registers with the `StatusNotifierWatcher`.
//! Two objects are served: `org.kde.StatusNotifierItem` for the icon, tooltip
//! and clicks, and `com.canonical.dbusmenu` for the menu. A desktop without a
//! watcher has no tray, and `build` says so instead of showing nothing.
//!
//! `zbus` answers the bus on an executor thread of its own, so the handler is
//! called from there.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use zbus::blocking::Connection;
use zbus::zvariant::{ObjectPath, OwnedValue, Structure, Value};

use super::Backend;
use super::flat_menu::{FlatMenu, Kind};
use crate::{TrayButton, TrayError, TrayEvent, TrayHandler, TrayIcon, TrayMenu};

const ITEM_PATH: &str = "/StatusNotifierItem";
const MENU_PATH: &str = "/MenuBar";
const ITEM_INTERFACE: &str = "org.kde.StatusNotifierItem";
const MENU_INTERFACE: &str = "com.canonical.dbusmenu";

static NEXT_TRAY: AtomicU32 = AtomicU32::new(1);

type Pixmap = (i32, i32, Vec<u8>);
type Layout = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);

struct State {
    icon: TrayIcon,
    tooltip: String,
    has_menu: bool,
    menu: FlatMenu,
    revision: u32,
}

#[derive(Clone)]
struct Shared {
    state: Arc<Mutex<State>>,
    handler: TrayHandler,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// The picture as the protocol wants it: ARGB32, most significant byte first.
fn pixmap(icon: &TrayIcon) -> Pixmap {
    let mut argb = Vec::with_capacity(icon.rgba().len());
    for pixel in icon.rgba().chunks_exact(4) {
        argb.extend_from_slice(&[pixel[3], pixel[0], pixel[1], pixel[2]]);
    }
    (icon.width() as i32, icon.height() as i32, argb)
}

fn owned(value: impl Into<Value<'static>>) -> OwnedValue {
    OwnedValue::try_from(value.into()).expect("a value without file descriptors always converts")
}

struct StatusNotifierItem(Shared);

#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl StatusNotifierItem {
    #[zbus(property)]
    fn category(&self) -> &str {
        "ApplicationStatus"
    }

    #[zbus(property)]
    fn id(&self) -> String {
        format!("aimer-tray-{}", std::process::id())
    }

    #[zbus(property)]
    fn title(&self) -> String {
        self.0.lock().tooltip.clone()
    }

    #[zbus(property)]
    fn status(&self) -> &str {
        "Active"
    }

    #[zbus(property)]
    fn window_id(&self) -> u32 {
        0
    }

    #[zbus(property)]
    fn icon_name(&self) -> &str {
        ""
    }

    #[zbus(property)]
    fn icon_pixmap(&self) -> Vec<Pixmap> {
        vec![pixmap(&self.0.lock().icon)]
    }

    #[zbus(property)]
    fn tool_tip(&self) -> (String, Vec<Pixmap>, String, String) {
        (String::new(), Vec::new(), self.0.lock().tooltip.clone(), String::new())
    }

    /// `false`: the host reports a plain click through [`Self::activate`].
    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
        false
    }

    /// `/` is how the protocol says "no menu".
    #[zbus(property)]
    fn menu(&self) -> ObjectPath<'static> {
        let path = if self.0.lock().has_menu { MENU_PATH } else { "/" };
        ObjectPath::from_static_str_unchecked(path)
    }

    fn activate(&self, _x: i32, _y: i32) {
        (self.0.handler)(TrayEvent::Click(TrayButton::Left));
    }

    fn secondary_activate(&self, _x: i32, _y: i32) {
        (self.0.handler)(TrayEvent::Click(TrayButton::Middle));
    }

    fn context_menu(&self, _x: i32, _y: i32) {
        (self.0.handler)(TrayEvent::Click(TrayButton::Right));
    }

    fn scroll(&self, _delta: i32, _orientation: &str) {}
}

struct DbusMenu(Shared);

/// The properties `dbusmenu` shows for row `id`.
fn properties(menu: &FlatMenu, id: i32) -> HashMap<String, OwnedValue> {
    let mut props = HashMap::new();
    let Some(node) = menu.node(id) else {
        // The root has no properties of its own, only children.
        props.insert("children-display".to_owned(), owned("submenu"));
        return props;
    };
    match node.kind {
        Kind::Separator => {
            props.insert("type".to_owned(), owned("separator"));
        }
        Kind::Item { enabled, checked, .. } => {
            props.insert("label".to_owned(), owned(node.label.clone()));
            props.insert("enabled".to_owned(), owned(enabled));
            if let Some(checked) = checked {
                props.insert("toggle-type".to_owned(), owned("checkmark"));
                props.insert("toggle-state".to_owned(), owned(i32::from(checked)));
            }
        }
        Kind::Submenu { enabled } => {
            props.insert("label".to_owned(), owned(node.label.clone()));
            props.insert("enabled".to_owned(), owned(enabled));
            props.insert("children-display".to_owned(), owned("submenu"));
        }
    }
    props
}

/// Row `id` and, while `depth` allows (`-1` is unlimited), everything below it.
fn layout(menu: &FlatMenu, id: i32, depth: i32) -> Option<Layout> {
    let children = menu.children_of(id)?;
    let nested = if depth == 0 {
        Vec::new()
    } else {
        children
            .iter()
            .filter_map(|&child| layout(menu, child, depth - 1))
            .map(|(id, props, kids)| owned(Structure::from((id, props, kids))))
            .collect()
    };
    Some((id, properties(menu, id), nested))
}

#[zbus::interface(name = "com.canonical.dbusmenu")]
impl DbusMenu {
    #[zbus(property)]
    fn version(&self) -> u32 {
        3
    }

    #[zbus(property)]
    fn text_direction(&self) -> &str {
        "ltr"
    }

    #[zbus(property)]
    fn status(&self) -> &str {
        "normal"
    }

    #[zbus(property)]
    fn icon_theme_path(&self) -> Vec<String> {
        Vec::new()
    }

    fn get_layout(
        &self,
        parent_id: i32,
        recursion_depth: i32,
        _property_names: Vec<String>,
    ) -> zbus::fdo::Result<(u32, Layout)> {
        let state = self.0.lock();
        let layout = layout(&state.menu, parent_id, recursion_depth)
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs(format!("no menu row {parent_id}")))?;
        Ok((state.revision, layout))
    }

    fn get_group_properties(&self, ids: Vec<i32>, _property_names: Vec<String>) -> Vec<(i32, HashMap<String, OwnedValue>)> {
        let state = self.0.lock();
        ids.into_iter().map(|id| (id, properties(&state.menu, id))).collect()
    }

    fn get_property(&self, id: i32, name: String) -> zbus::fdo::Result<OwnedValue> {
        properties(&self.0.lock().menu, id)
            .remove(&name)
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs(format!("row {id} has no property {name}")))
    }

    fn event(&self, id: i32, event_id: &str, _data: Value<'_>, _timestamp: u32) {
        if event_id != "clicked" {
            return;
        }
        let item = self.0.lock().menu.item_id(id);
        if let Some(item) = item {
            (self.0.handler)(TrayEvent::MenuItem(item));
        }
    }

    fn event_group(&self, events: Vec<(i32, String, Value<'_>, u32)>) -> Vec<i32> {
        for (id, event_id, data, timestamp) in events {
            self.event(id, &event_id, data, timestamp);
        }
        Vec::new()
    }

    fn about_to_show(&self, _id: i32) -> bool {
        false
    }

    fn about_to_show_group(&self, _ids: Vec<i32>) -> (Vec<i32>, Vec<i32>) {
        (Vec::new(), Vec::new())
    }
}

struct LinuxTray {
    connection: Connection,
    shared: Shared,
}

fn platform(error: impl std::fmt::Display) -> TrayError {
    TrayError::Platform(error.to_string())
}

pub(super) fn create(
    icon: &TrayIcon,
    tooltip: &str,
    menu: Option<&TrayMenu>,
    handler: TrayHandler,
) -> Result<Box<dyn Backend>, TrayError> {
    let shared = Shared {
        state: Arc::new(Mutex::new(State {
            icon: icon.clone(),
            tooltip: tooltip.to_owned(),
            has_menu: menu.is_some(),
            menu: menu.map(FlatMenu::from_menu).unwrap_or_default(),
            revision: 1,
        })),
        handler,
    };
    let connection = Connection::session().map_err(platform)?;
    {
        let objects = connection.object_server();
        objects.at(ITEM_PATH, StatusNotifierItem(shared.clone())).map_err(platform)?;
        objects.at(MENU_PATH, DbusMenu(shared.clone())).map_err(platform)?;
    }
    let name = format!("org.kde.StatusNotifierItem-{}-{}", std::process::id(), NEXT_TRAY.fetch_add(1, Ordering::Relaxed));
    connection.request_name(name.as_str()).map_err(platform)?;
    connection
        .call_method(
            Some("org.kde.StatusNotifierWatcher"),
            "/StatusNotifierWatcher",
            Some("org.kde.StatusNotifierWatcher"),
            "RegisterStatusNotifierItem",
            &name.as_str(),
        )
        .map_err(|error| TrayError::Platform(format!("this desktop has no system tray host: {error}")))?;
    Ok(Box::new(LinuxTray { connection, shared }))
}

impl LinuxTray {
    /// Tells the host that a property of the item changed. A failed signal only
    /// means the host refreshes later; there is nobody to tell.
    fn announce_item(&self, signal: &str) {
        let _ = self.connection.emit_signal(None::<&str>, ITEM_PATH, ITEM_INTERFACE, signal, &());
    }

    fn announce_layout(&self, revision: u32) {
        let _ = self.connection.emit_signal(None::<&str>, MENU_PATH, MENU_INTERFACE, "LayoutUpdated", &(revision, 0i32));
    }
}

impl Backend for LinuxTray {
    fn set_icon(&mut self, icon: &TrayIcon) {
        self.shared.lock().icon = icon.clone();
        self.announce_item("NewIcon");
    }

    fn set_tooltip(&mut self, tooltip: &str) {
        self.shared.lock().tooltip = tooltip.to_owned();
        self.announce_item("NewToolTip");
        self.announce_item("NewTitle");
    }

    fn set_menu(&mut self, menu: Option<&TrayMenu>) {
        let revision = {
            let mut state = self.shared.lock();
            state.has_menu = menu.is_some();
            state.menu = menu.map(FlatMenu::from_menu).unwrap_or_default();
            state.revision = state.revision.wrapping_add(1);
            state.revision
        };
        self.announce_layout(revision);
    }
}
