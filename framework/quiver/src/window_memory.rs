//! Windows that remember where they were.
//!
//! A window built with [`WindowAttr::remember`](aimer_window::WindowAttr::remember)
//! comes back as the user left it: at its position, at its size, maximized or
//! full screen if it was. The state is kept in memory for the windows of this
//! run, and in one small text file in the application's data directory for the
//! next one.
//!
//! The size is logical and the position physical, as winit reports them: a
//! window restored on a screen with another scale keeps the size it looked,
//! and lands where the pixels were. A position that is on none of the current
//! screens is dropped, so a monitor that has been unplugged cannot hide a
//! window.
//!
//! A maximized, full-screen or minimized window does not describe its normal
//! size, so none of those overwrite the rectangle that was remembered: leaving
//! maximized later gives back the window the user had before.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use aimer_utils::info;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event_loop::ActiveEventLoop;
use winit::window::{Fullscreen, Window, WindowAttributes};

/// The smallest logical size a remembered window is restored with, so a state
/// written by a collapsed window cannot bring back one the user cannot grab.
const MIN_SIZE: (u32, u32) = (200, 120);

/// How far below the top of a window the row that must be on a screen is: a
/// window is reachable as long as its title bar is.
const GRAB_DEPTH: i32 = 16;

/// How much of a window's width must be over a screen for it to be grabbed.
const GRAB_WIDTH: i64 = 64;

/// Where a window was, and what it was doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WindowGeometry {
    /// Physical outer position.
    pub(crate) x: i32,
    pub(crate) y: i32,
    /// Logical inner size.
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) maximized: bool,
    pub(crate) fullscreen: bool,
}

impl WindowGeometry {
    /// `x y width height maximized fullscreen`, the part of a line after the
    /// name.
    fn encode(&self) -> String {
        format!(
            "{} {} {} {} {} {}",
            self.x, self.y, self.width, self.height, self.maximized as u8, self.fullscreen as u8
        )
    }

    fn decode(text: &str) -> Option<Self> {
        let mut fields = text.split(' ');
        let x = fields.next()?.parse().ok()?;
        let y = fields.next()?.parse().ok()?;
        let width = fields.next()?.parse().ok()?;
        let height = fields.next()?.parse().ok()?;
        let flag = |field: Option<&str>| match field? {
            "0" => Some(false),
            "1" => Some(true),
            _ => None,
        };
        let maximized = flag(fields.next())?;
        let fullscreen = flag(fields.next())?;
        if fields.next().is_some() || width == 0 || height == 0 {
            return None;
        }
        Some(Self { x, y, width, height, maximized, fullscreen })
    }

    /// What a window looks like now. A window that is maximized, full screen
    /// or minimized keeps the rectangle `previous` had, since the one it has
    /// now is not the one it will return to.
    fn observe(
        current: (i32, i32, u32, u32),
        maximized: bool,
        fullscreen: bool,
        minimized: bool,
        previous: Option<Self>,
    ) -> Self {
        let (x, y, width, height) = current;
        let normal = Self { x, y, width, height, maximized, fullscreen };
        match previous {
            Some(previous) if maximized || fullscreen || minimized => {
                Self { maximized, fullscreen, ..previous }
            }
            // A minimized window with nothing before it has no usable
            // rectangle either (Windows parks it far off screen); the flags
            // are all there is to keep.
            None if minimized => Self { maximized, fullscreen, ..normal },
            _ => normal,
        }
    }

    /// Whether the window's title bar can be grabbed on one of `screens`, each
    /// a physical `(x, y, width, height)`: a row of it below the top edge is on
    /// the screen, and enough of its width is over it. A window partly off an
    /// edge is still reachable. Without any screen to compare with, nothing can
    /// be said to be off them.
    fn is_reachable(&self, screens: &[(i32, i32, u32, u32)]) -> bool {
        if screens.is_empty() {
            return true;
        }
        let (x, width) = (i64::from(self.x), i64::from(self.width));
        let row = i64::from(self.y) + i64::from(GRAB_DEPTH);
        let needed = width.min(GRAB_WIDTH);
        screens.iter().any(|&(sx, sy, sw, sh)| {
            let (sx, sy) = (i64::from(sx), i64::from(sy));
            let overlap = (x + width).min(sx + i64::from(sw)) - x.max(sx);
            overlap >= needed && row >= sy && row < sy + i64::from(sh)
        })
    }
}

/// The remembered windows, by name, in the order they were first seen.
#[derive(Debug, Default, PartialEq, Eq)]
struct Memory {
    entries: Vec<(String, WindowGeometry)>,
}

impl Memory {
    fn get(&self, name: &str) -> Option<WindowGeometry> {
        self.entries.iter().find(|(key, _)| key == name).map(|(_, geometry)| *geometry)
    }

    fn set(&mut self, name: &str, geometry: WindowGeometry) {
        match self.entries.iter_mut().find(|(key, _)| key == name) {
            Some((_, slot)) => *slot = geometry,
            None => self.entries.push((name.to_owned(), geometry)),
        }
    }

    /// One `name<TAB>fields` line per window. A name cannot hold a tab or a
    /// line break, so they are replaced when the name is used.
    fn render(&self) -> String {
        let mut text = String::new();
        for (name, geometry) in &self.entries {
            text.push_str(name);
            text.push('\t');
            text.push_str(&geometry.encode());
            text.push('\n');
        }
        text
    }

    /// Reads what [`render`](Self::render) wrote. A line that does not make
    /// sense is skipped rather than failing the others.
    fn parse(text: &str) -> Self {
        let mut memory = Self::default();
        for line in text.lines() {
            let Some((name, fields)) = line.split_once('\t') else {
                continue;
            };
            if let Some(geometry) = WindowGeometry::decode(fields) {
                memory.set(name, geometry);
            }
        }
        memory
    }
}

/// A name as it is stored: one line, no tab.
fn storable(name: &str) -> String {
    name.chars().map(|c| if c == '\t' || c.is_control() { '_' } else { c }).collect()
}

/// The file the state of this application is kept in. `None` where the
/// platform has no data directory or the executable has no usable name.
fn state_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let app_id = exe.file_stem()?.to_str()?.to_owned();
    Some(aimer_storage::app_data_dir(&app_id).ok()?.join("window-state.txt"))
}

fn load(path: &Path) -> Memory {
    std::fs::read_to_string(path).map(|text| Memory::parse(&text)).unwrap_or_default()
}

/// Writes through a sibling file and a rename, so a crash cannot leave half a
/// state behind.
fn store(path: &Path, memory: &Memory) -> std::io::Result<()> {
    if let Some(directory) = path.parent() {
        std::fs::create_dir_all(directory)?;
    }
    let temporary = path.with_extension("txt.tmp");
    std::fs::write(&temporary, memory.render())?;
    std::fs::rename(&temporary, path)
}

std::thread_local! {
    /// Loaded on first use. Windows live on the UI thread, so is every caller.
    static MEMORY: RefCell<Option<Memory>> = const { RefCell::new(None) };
}

fn with_memory<R>(f: impl FnOnce(&mut Memory) -> R) -> R {
    MEMORY.with_borrow_mut(|slot| {
        let memory = slot.get_or_insert_with(|| state_path().map(|path| load(&path)).unwrap_or_default());
        f(memory)
    })
}

fn screens(event_loop: &ActiveEventLoop) -> Vec<(i32, i32, u32, u32)> {
    event_loop
        .available_monitors()
        .map(|monitor| {
            let (position, size) = (monitor.position(), monitor.size());
            (position.x, position.y, size.width, size.height)
        })
        .collect()
}

/// Gives `attributes` the state remembered under `name`, if there is one.
///
/// `resizable` is whether the window may change size at all: a fixed window
/// keeps the size it was built with.
pub(crate) fn restore(
    attributes: WindowAttributes,
    name: &str,
    resizable: bool,
    event_loop: &ActiveEventLoop,
) -> WindowAttributes {
    let Some(geometry) = with_memory(|memory| memory.get(&storable(name))) else {
        return attributes;
    };
    apply(attributes, &geometry, resizable, &screens(event_loop))
}

fn apply(
    mut attributes: WindowAttributes,
    geometry: &WindowGeometry,
    resizable: bool,
    screens: &[(i32, i32, u32, u32)],
) -> WindowAttributes {
    if resizable {
        let (width, height) = (geometry.width.max(MIN_SIZE.0), geometry.height.max(MIN_SIZE.1));
        attributes = attributes.with_inner_size(LogicalSize::new(width, height));
    }
    if geometry.is_reachable(screens) {
        attributes = attributes.with_position(PhysicalPosition::new(geometry.x, geometry.y));
    }
    if geometry.fullscreen {
        attributes = attributes.with_fullscreen(Some(Fullscreen::Borderless(None)));
    }
    attributes.with_maximized(geometry.maximized && resizable)
}

/// Remembers `window` under `name`, in memory and on disk.
///
/// Called when the window closes and when the application ends. A failure to
/// write is logged and otherwise ignored: the window just opens at its default
/// next time.
pub(crate) fn remember(name: &str, window: &Window) {
    let Ok(position) = window.outer_position() else {
        return;
    };
    let size = window.inner_size().to_logical::<f64>(window.scale_factor());
    let current = (position.x, position.y, size.width.round() as u32, size.height.round() as u32);
    let name = storable(name);
    with_memory(|memory| {
        let geometry = WindowGeometry::observe(
            current,
            window.is_maximized(),
            window.fullscreen().is_some(),
            window.is_minimized().unwrap_or(false),
            memory.get(&name),
        );
        if geometry.width == 0 || geometry.height == 0 {
            return;
        }
        memory.set(&name, geometry);
        if let Some(path) = state_path() {
            if let Err(error) = store(&path, memory) {
                info!("window state not saved to {}: {error}", path.display());
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geometry(x: i32, y: i32, width: u32, height: u32) -> WindowGeometry {
        WindowGeometry { x, y, width, height, maximized: false, fullscreen: false }
    }

    const SCREEN: (i32, i32, u32, u32) = (0, 0, 2560, 1440);

    #[test]
    fn a_geometry_survives_being_written_and_read() {
        let original = WindowGeometry { x: -120, y: 48, width: 1280, height: 800, maximized: true, fullscreen: false };
        assert_eq!(WindowGeometry::decode(&original.encode()), Some(original));
    }

    #[test]
    fn a_geometry_that_makes_no_sense_is_refused() {
        assert_eq!(WindowGeometry::decode(""), None);
        assert_eq!(WindowGeometry::decode("1 2 3"), None);
        assert_eq!(WindowGeometry::decode("1 2 3 4 5 0"), None, "a flag is 0 or 1");
        assert_eq!(WindowGeometry::decode("1 2 3 4 0 0 9"), None, "nothing follows the flags");
        assert_eq!(WindowGeometry::decode("1 2 0 4 0 0"), None, "a window has an area");
        assert_eq!(WindowGeometry::decode("a 2 3 4 0 0"), None);
    }

    #[test]
    fn the_memory_round_trips_and_skips_the_lines_it_cannot_read() {
        let mut memory = Memory::default();
        memory.set("main", geometry(10, 20, 900, 600));
        memory.set("inspector", geometry(-5, 0, 300, 400));
        let mut text = memory.render();
        text.push_str("garbage line\nbroken\t1 2\n");

        assert_eq!(Memory::parse(&text), memory);
    }

    #[test]
    fn setting_a_name_again_replaces_it_in_place() {
        let mut memory = Memory::default();
        memory.set("main", geometry(0, 0, 400, 300));
        memory.set("other", geometry(1, 1, 400, 300));
        memory.set("main", geometry(9, 9, 800, 600));

        assert_eq!(memory.get("main"), Some(geometry(9, 9, 800, 600)));
        assert_eq!(memory.entries.len(), 2);
        assert_eq!(memory.entries[0].0, "main");
    }

    #[test]
    fn a_name_is_stored_on_one_line() {
        assert_eq!(storable("a\tb\nc"), "a_b_c");
        assert_eq!(storable("main"), "main");
    }

    #[test]
    fn a_normal_window_is_remembered_as_it_is() {
        let seen = WindowGeometry::observe((5, 6, 700, 500), false, false, false, Some(geometry(0, 0, 1, 1)));
        assert_eq!(seen, geometry(5, 6, 700, 500));
    }

    #[test]
    fn a_maximized_window_keeps_the_rectangle_it_will_return_to() {
        let before = geometry(100, 80, 900, 600);

        let seen = WindowGeometry::observe((0, 0, 2560, 1400), true, false, false, Some(before));

        assert_eq!(seen, WindowGeometry { maximized: true, ..before });
    }

    #[test]
    fn a_full_screen_or_minimized_window_keeps_it_too() {
        let before = geometry(100, 80, 900, 600);

        let full = WindowGeometry::observe((0, 0, 2560, 1440), false, true, false, Some(before));
        let parked = WindowGeometry::observe((-32000, -32000, 160, 28), false, false, true, Some(before));

        assert_eq!(full, WindowGeometry { fullscreen: true, ..before });
        assert_eq!(parked, before);
    }

    #[test]
    fn a_maximized_window_with_no_history_is_remembered_with_what_it_has() {
        let seen = WindowGeometry::observe((0, 0, 2560, 1400), true, false, false, None);
        assert_eq!(seen, WindowGeometry { maximized: true, ..geometry(0, 0, 2560, 1400) });
    }

    #[test]
    fn a_window_is_reachable_by_its_title_bar() {
        assert!(geometry(100, 100, 800, 600).is_reachable(&[SCREEN]));
        // Mostly off the edge, with its title bar still on screen.
        assert!(geometry(2000, 100, 1200, 600).is_reachable(&[SCREEN]));
        // Only a sliver left over the screen: nothing to grab.
        assert!(!geometry(2540, 100, 800, 600).is_reachable(&[SCREEN]));
        // Title bar above the top of the screen.
        assert!(!geometry(100, -200, 800, 600).is_reachable(&[SCREEN]));
        // On a monitor that is gone.
        assert!(!geometry(3000, 100, 800, 600).is_reachable(&[SCREEN]));
        assert!(!geometry(-32000, -32000, 800, 600).is_reachable(&[SCREEN]));
        // A second monitor to the left, at negative coordinates.
        assert!(geometry(-1500, 200, 800, 600).is_reachable(&[(-1920, 0, 1920, 1080), SCREEN]));
    }

    #[test]
    fn nothing_is_off_screen_when_there_are_no_screens_to_check() {
        assert!(geometry(99999, 99999, 800, 600).is_reachable(&[]));
    }

    #[test]
    fn restoring_sets_size_position_and_maximized() {
        let saved = WindowGeometry { maximized: true, ..geometry(40, 50, 1000, 700) };

        let attributes = apply(WindowAttributes::default(), &saved, true, &[SCREEN]);

        assert_eq!(attributes.inner_size, Some(LogicalSize::new(1000, 700).into()));
        assert_eq!(attributes.position, Some(PhysicalPosition::new(40, 50).into()));
        assert!(attributes.maximized);
    }

    #[test]
    fn restoring_drops_an_unreachable_position_but_keeps_the_size() {
        let attributes = apply(WindowAttributes::default(), &geometry(9000, 50, 1000, 700), true, &[SCREEN]);

        assert_eq!(attributes.position, None);
        assert_eq!(attributes.inner_size, Some(LogicalSize::new(1000, 700).into()));
    }

    #[test]
    fn restoring_never_makes_a_window_too_small_to_grab() {
        let attributes = apply(WindowAttributes::default(), &geometry(0, 0, 10, 10), true, &[SCREEN]);

        assert_eq!(attributes.inner_size, Some(LogicalSize::new(MIN_SIZE.0, MIN_SIZE.1).into()));
    }

    #[test]
    fn a_fixed_size_window_is_not_resized_or_maximized() {
        let saved = WindowGeometry { maximized: true, ..geometry(0, 0, 1000, 700) };

        let attributes = apply(WindowAttributes::default(), &saved, false, &[SCREEN]);

        assert_eq!(attributes.inner_size, None);
        assert!(!attributes.maximized);
    }

    #[test]
    fn the_state_file_round_trips_through_the_disk() {
        let directory = std::env::temp_dir().join(format!("aimer-window-memory-{}", std::process::id()));
        let path = directory.join("nested").join("window-state.txt");
        let mut memory = Memory::default();
        memory.set("main", geometry(1, 2, 640, 480));

        store(&path, &memory).expect("the directory is created and the file written");

        assert_eq!(load(&path), memory);
        assert_eq!(load(&directory.join("missing.txt")), Memory::default());
        let _ = std::fs::remove_dir_all(&directory);
    }
}
