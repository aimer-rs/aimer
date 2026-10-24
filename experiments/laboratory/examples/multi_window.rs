//! Several windows on one event loop.
//!
//! Every window has a counter of its own: clicking in one never changes
//! another, and each rebuilds and repaints on its own. Any window can open
//! more with `NewWindow`, any window can close itself, and the first one also
//! declares a settings window with the `Window` widget, shown while a piece of
//! its state says so. The application ends when the last window does.
//!
//! ```text
//! cargo run -p aimer_laboratory --example multi_window
//! ```
//!
//! With `AIMER_MULTI_WINDOW_SMOKE` set, the example drives itself on a timer
//! and prints what the host logs, so it can run unattended. The value picks
//! what is exercised:
//!
//! - `1` opens and closes two windows and quits.
//! - `main` closes the first window and expects the others to go with it.
//! - `never` closes every window and expects the application to stay alive
//!   until it is told to quit.
//! - `declarative` shows, retitles, hides and shows again the settings window
//!   by changing the first window's state, and then quits.
//! - `churn` opens and closes windows one after another (30 by default,
//!   `AIMER_CHURN_ROUNDS` changes it) and prints the process's memory and
//!   thread count along the way, so a window that is not freed shows up as
//!   growth. `AIMER_CHURN_IDLE_MS` adds a final reading after an idle wait.
//!
//! ```text
//! AIMER_MULTI_WINDOW_SMOKE=declarative cargo run -p aimer_laboratory --example multi_window
//! ```

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use aimer::{
    AimerApp, AnyElement, BuildContext, Column, ExitPolicy, NewWindow, State, StateUpdater,
    StatefulElement, StatefulWidget, Text, TextButton, Widget, Window, WindowAttr, WindowId,
};

/// What every window shares: how many have been opened, for their titles.
#[derive(Clone, Default)]
struct Opened(Rc<Cell<usize>>);

/// How a script reaches the first window's state.
type Controls = Rc<RefCell<Option<StateUpdater<PanelState>>>>;

struct Panel {
    number: usize,
    opened: Opened,
    /// Where this window's id goes once it has one, so it can close itself.
    id: Rc<Cell<Option<WindowId>>>,
    controls: Controls,
    /// Whether this window declares the settings window.
    settings: bool,
}

struct PanelState {
    number: usize,
    opened: Opened,
    id: Rc<Cell<Option<WindowId>>>,
    controls: Controls,
    settings: bool,
    clicks: u32,
    settings_open: bool,
    settings_title: &'static str,
    updater: Option<StateUpdater<PanelState>>,
}

impl StatefulWidget for Panel {
    type State = PanelState;

    fn create_state(self) -> PanelState {
        PanelState {
            number: self.number,
            opened: self.opened,
            id: self.id,
            controls: self.controls,
            settings: self.settings,
            clicks: 0,
            settings_open: false,
            settings_title: "Settings",
            updater: None,
        }
    }
}

impl State<Panel> for PanelState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.updater = Some(updater);
        if self.settings {
            *self.controls.borrow_mut() = Some(updater);
        }
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        let updater = self.updater.expect("initialised before the first build");
        let opened = self.opened.clone();
        let id = self.id.clone();
        let controls = self.controls.clone();

        let mut children = vec![
            Text::new(format!(
                "Window {} - clicked {} times",
                self.number, self.clicks
            ))
            .boxed(),
            TextButton::new("Click")
                .on_press(move || updater.set_state(|state| state.clicks += 1))
                .boxed(),
            TextButton::new("Open another window")
                .on_press(move || {
                    let number = opened.0.get() + 1;
                    opened.0.set(number);
                    let window = open_numbered(number, &opened, &controls);
                    eprintln!("opened window {number}: {window:?}");
                })
                .boxed(),
            TextButton::new("Close this window")
                .on_press(move || {
                    if let Some(id) = id.get() {
                        id.close();
                    }
                })
                .boxed(),
        ];

        if self.settings {
            let shown = self.settings_open;
            children.push(
                TextButton::new(if shown {
                    "Hide settings"
                } else {
                    "Show settings"
                })
                .on_press(move || {
                    updater.set_state(|state| state.settings_open = !state.settings_open);
                })
                .boxed(),
            );
            // Declared here, shown by the state above. When the user closes the
            // window itself, the state is told so the button stays truthful.
            children.push(
                Window::new()
                    .title(self.settings_title)
                    .size(360, 200)
                    .open(self.settings_open)
                    .on_close(move || updater.set_state(|state| state.settings_open = false))
                    .builder(|_ctx| Text::new("Settings live in a window of their own"))
                    .boxed(),
            );
        }

        Column::new().children(children)
    }
}

impl Widget for Panel {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "MultiWindowPanel", None)
            .0
            .boxed()
    }
}

impl aimer::PortableWidget for Panel {}

fn attr(number: usize) -> WindowAttr {
    WindowAttr::new()
        .title(format!("Window {number}"))
        .inner_size(420, 260)
}

/// Opens window `number`, the way a button handler would.
///
/// The window's content builder runs more than once and a widget is consumed by
/// building it, so each build makes the panel again from what it is made of.
fn open_numbered(number: usize, opened: &Opened, controls: &Controls) -> WindowId {
    let id = Rc::new(Cell::new(None));
    let panel_id = id.clone();
    let opened = opened.clone();
    let controls = controls.clone();
    let window = NewWindow::new()
        .attr(attr(number))
        .builder(move |_ctx| Panel {
            number,
            opened: opened.clone(),
            id: panel_id.clone(),
            controls: controls.clone(),
            settings: false,
        })
        .open();
    id.set(Some(window));
    window
}

/// Drives the application on a timer and then quits, so the host can be
/// exercised without anyone at the keyboard. The host logs each step.
async fn smoke_script(opened: Opened, controls: Controls, mode: String) {
    use tokio::time::{Duration, sleep};

    if mode == "declarative" {
        // The settings window is shown, retitled, hidden and shown again by the
        // state of the first window alone.
        let set = |change: fn(&mut PanelState)| {
            let updater = controls.borrow().expect("the first window was built");
            updater.set_state(change);
        };
        sleep(Duration::from_millis(1000)).await;
        eprintln!("smoke: showing the settings window");
        set(|state| state.settings_open = true);
        sleep(Duration::from_millis(1000)).await;
        eprintln!("smoke: retitling it");
        set(|state| state.settings_title = "Preferences");
        sleep(Duration::from_millis(1000)).await;
        eprintln!("smoke: hiding it");
        set(|state| state.settings_open = false);
        sleep(Duration::from_millis(1000)).await;
        eprintln!("smoke: showing it again");
        set(|state| state.settings_open = true);
        sleep(Duration::from_millis(1000)).await;
        eprintln!("smoke: quitting with it open");
        aimer::quiver::windows::quit();
        return;
    }
    if mode == "main" {
        // Closing the first window takes every other with it.
        sleep(Duration::from_millis(800)).await;
        open_numbered(2, &opened, &controls);
        open_numbered(3, &opened, &controls);
        sleep(Duration::from_millis(1000)).await;
        eprintln!("smoke: closing the main window");
        WindowId::MAIN.close();
        // The loop must end by itself; if it has not, say so and stop it.
        sleep(Duration::from_millis(3000)).await;
        eprintln!("smoke: FAILED, the application is still running");
        aimer::quiver::windows::quit();
        return;
    }
    if mode == "churn" {
        // Opens and closes many windows one after another and reports how much
        // memory the process holds before and after. A window that is not freed
        // when it closes shows up as growth.
        let resident = || {
            let output = std::process::Command::new("ps")
                .args(["-o", "rss=", "-p", &std::process::id().to_string()])
                .output()
                .ok();
            output
                .and_then(|output| String::from_utf8(output.stdout).ok())
                .and_then(|text| text.trim().parse::<u64>().ok())
                .unwrap_or(0)
        };
        let threads = || {
            let output = std::process::Command::new("ps")
                .args(["-M", "-p", &std::process::id().to_string()])
                .output()
                .ok();
            output
                .and_then(|output| String::from_utf8(output.stdout).ok())
                .map(|text| text.lines().count().saturating_sub(1))
                .unwrap_or(0)
        };
        sleep(Duration::from_millis(800)).await;
        let rounds = std::env::var("AIMER_CHURN_ROUNDS")
            .ok()
            .and_then(|rounds| rounds.parse::<usize>().ok())
            .unwrap_or(30);
        for round in 0..rounds {
            let window = open_numbered(100 + round, &opened, &controls);
            sleep(Duration::from_millis(250)).await;
            window.close();
            sleep(Duration::from_millis(150)).await;
            if (round + 1) % 20 == 0 || round == 4 {
                eprintln!(
                    "smoke: after {} windows: {} KiB resident, {} threads",
                    round + 1,
                    resident(),
                    threads()
                );
            }
        }
        eprintln!(
            "smoke: after {rounds} windows: {} KiB resident, {} threads",
            resident(),
            threads()
        );
        // Let the system reclaim whatever it is going to before the last count.
        let idle = std::env::var("AIMER_CHURN_IDLE_MS")
            .ok()
            .and_then(|idle| idle.parse::<u64>().ok())
            .unwrap_or(0);
        if idle > 0 {
            sleep(Duration::from_millis(idle)).await;
            eprintln!(
                "smoke: after {idle} ms idle: {} KiB resident, {} threads",
                resident(),
                threads()
            );
        }
        aimer::quiver::windows::quit();
        return;
    }
    if mode == "never" {
        // Closing every window leaves the application running.
        sleep(Duration::from_millis(800)).await;
        let second = open_numbered(2, &opened, &controls);
        sleep(Duration::from_millis(500)).await;
        second.close();
        WindowId::MAIN.close();
        sleep(Duration::from_millis(1500)).await;
        eprintln!("smoke: no windows left, still running");
        open_numbered(4, &opened, &controls);
        sleep(Duration::from_millis(800)).await;
        eprintln!("smoke: a window opened with none left, quitting");
        aimer::quiver::windows::quit();
        return;
    }

    sleep(Duration::from_millis(1000)).await;
    let a = open_numbered(2, &opened, &controls);
    eprintln!("smoke: asked for window 2");

    sleep(Duration::from_millis(500)).await;
    let b = open_numbered(3, &opened, &controls);
    eprintln!("smoke: asked for window 3");

    sleep(Duration::from_millis(1000)).await;
    a.close();
    eprintln!("smoke: asked window 2 to close");

    sleep(Duration::from_millis(500)).await;
    b.close();
    eprintln!("smoke: asked window 3 to close");

    sleep(Duration::from_millis(500)).await;
    eprintln!("smoke: quitting");
    aimer::quiver::windows::quit();
}

fn main() {
    let opened = Opened::default();
    opened.0.set(3);
    let controls = Controls::default();
    let mode = std::env::var("AIMER_MULTI_WINDOW_SMOKE").ok();
    let policy = match mode.as_deref() {
        Some("main") => ExitPolicy::MainWindowClosed,
        Some("never") => ExitPolicy::Never,
        _ => ExitPolicy::LastWindowClosed,
    };
    let first = Panel {
        number: 1,
        opened: opened.clone(),
        id: Rc::new(Cell::new(Some(WindowId::MAIN))),
        controls: controls.clone(),
        settings: true,
    };
    let app = AimerApp::new()
        .window(attr(1))
        .exit_policy(policy)
        .child(first);

    if let Some(mode) = mode {
        app.run_with_setup(move || {
            aimer::Venus::current()
                .expect("the application's runtime is installed before setup")
                .spawn(smoke_script(opened, controls, mode));
        });
    } else {
        app.run();
    }
}
