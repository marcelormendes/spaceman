//! Setup tour orchestration and live polling.

mod desk;
mod flow;
#[cfg(target_os = "macos")]
mod keys;
mod sandbox;
#[cfg(test)]
mod tests;
mod view;

use std::time::{Duration, Instant};

pub use flow::{Effect, Flow, Input, SetupState, Step, launch_step};
pub use sandbox::{Key, Sandbox};
pub use view::overlay;

use rune::Tx;
use spaceman::app::UiLaunchAtLogin;

use crate::{
    Pane, Ui, apps,
    playground::{Scene, reset_fixture},
};

const ADVANCE_BEAT: Duration = Duration::from_millis(700);
/// How long after a real shortcut the setup window comes back to the front, so the app that
/// shortcut brought forward settles first.
#[cfg(target_os = "macos")]
const RAISE_AFTER: Duration = Duration::from_millis(350);

pub struct Tour {
    pub flow: Flow,
    pub conflicts: Vec<String>,
    /// The Done step's practice desktop.
    pub sandbox: Sandbox,
    advance_at: Option<Instant>,
    completed: bool,
    /// Hears the practice shortcuts while the Done step shows in a live window.
    #[cfg(target_os = "macos")]
    keys: Option<keys::Listener>,
    /// When to bring the setup window back after a shortcut that also ran for real.
    #[cfg(target_os = "macos")]
    raise_at: Option<Instant>,
}

impl Tour {
    pub fn from_launch(step: Step, accessibility: bool, ui: &mut Ui) -> Self {
        let status = SetupState {
            accessibility,
            launch_at_login: ui_launch_at_login(ui),
        };
        Self {
            flow: Flow::new(step, status, false),
            conflicts: apps::conflicting_window_managers(),
            sandbox: Sandbox::default(),
            advance_at: None,
            completed: false,
            #[cfg(target_os = "macos")]
            keys: None,
            #[cfg(target_os = "macos")]
            raise_at: None,
        }
    }

    pub fn manual(ui: &mut Ui) -> Self {
        let status = ui_setup_state(ui);
        Self {
            flow: Flow::new(Step::Welcome, status, true),
            conflicts: apps::conflicting_window_managers(),
            sandbox: Sandbox::default(),
            advance_at: None,
            completed: true,
            #[cfg(target_os = "macos")]
            keys: None,
            #[cfg(target_os = "macos")]
            raise_at: None,
        }
    }

    fn fixture(
        step: Step,
        accessibility: bool,
        asked: bool,
        conflicts: Vec<String>,
        ui: &mut Ui,
    ) -> Self {
        if let Some(agent) = &mut ui.agent {
            agent.accessibility = accessibility;
            // A first-run tour holds tiling until it finishes.
            agent.automatic = false;
        }
        let status = SetupState {
            accessibility,
            launch_at_login: ui_launch_at_login(ui),
        };
        let mut flow = Flow::new(Step::Welcome, status, false);
        while flow.step() != step {
            if flow.step() == Step::Setup && !accessibility {
                // Force fixture into later step only when accessibility is true.
                break;
            }
            flow.on(Input::Continue);
        }
        if step == Step::Setup && asked && !accessibility {
            flow.on(Input::GrantAccessibility);
        }
        Self {
            flow,
            conflicts,
            sandbox: Sandbox::default(),
            advance_at: None,
            completed: false,
            #[cfg(target_os = "macos")]
            keys: None,
            #[cfg(target_os = "macos")]
            raise_at: None,
        }
    }
}

pub fn poll(ui: &mut Ui, tx: &mut Tx) -> bool {
    let status = ui_setup_state(ui);
    let live = !ui.fixture;
    let Some(tour) = &mut ui.tour else {
        return false;
    };
    let before = tour.flow.view();
    let step_before = before.step;
    let mut effect = tour.flow.on(Input::Status(status));
    if tour
        .advance_at
        .is_some_and(|at| at.elapsed() >= ADVANCE_BEAT)
    {
        tour.advance_at = None;
        if tour.flow.step() == Step::Setup {
            effect = tour.flow.on(Input::Continue);
        }
    }
    if status.accessibility && !tour.completed {
        tour.completed = true;
        if live {
            flow::record_completion();
        }
    }
    apply(ui, effect);
    remember_step(ui, step_before);
    listen_on_done(ui);
    raise_after_shortcut(ui);
    if let Some(at) = ui.tour.as_ref().and_then(|tour| tour.advance_at) {
        tx.wake_after(
            ADVANCE_BEAT
                .saturating_sub(at.elapsed())
                .max(Duration::from_millis(1)),
        );
    }
    let after = ui.tour.as_ref().map(|tour| tour.flow.view());
    after.as_ref() != Some(&before)
}

pub fn input(ui: &mut Ui, input: Input) {
    let Some(tour) = &mut ui.tour else {
        return;
    };
    let step_before = tour.flow.step();
    let effect = tour.flow.on(input);
    apply(ui, effect);
    remember_step(ui, step_before);
    listen_on_done(ui);
}

/// Plays a practice shortcut on the Done step's desktop. Returns whether the view changed.
pub fn press(ui: &mut Ui, key: Key) -> bool {
    match &mut ui.tour {
        Some(tour) if tour.flow.step() == Step::Done => {
            tour.sandbox.press(key);
            true
        }
        _ => false,
    }
}

/// Listens for the practice shortcuts only while a live window shows the Done step.
#[cfg(target_os = "macos")]
fn listen_on_done(ui: &mut Ui) {
    let window = (!ui.fixture).then(|| ui.window.clone()).flatten();
    let Some(tour) = &mut ui.tour else {
        return;
    };
    if tour.flow.step() != Step::Done {
        tour.keys = None;
    } else if tour.keys.is_none()
        && let Some(window) = window
    {
        tour.keys = keys::listen(window);
    }
}

#[cfg(not(target_os = "macos"))]
fn listen_on_done(_: &mut Ui) {}

/// A shortcut the key tap heard. With tiling already on, it also ran for real and may have
/// brought another app forward, so the setup window comes back once that settles.
#[cfg(target_os = "macos")]
fn heard(ui: &mut Ui, key: Key, tx: &mut Tx) -> bool {
    let tiling = !ui.fixture && ui.agent.as_ref().is_some_and(|agent| agent.automatic);
    let changed = press(ui, key);
    if changed
        && tiling
        && let Some(tour) = &mut ui.tour
    {
        tour.raise_at = Some(Instant::now() + RAISE_AFTER);
        tx.wake_after(RAISE_AFTER);
    }
    changed
}

#[cfg(target_os = "macos")]
fn raise_after_shortcut(ui: &mut Ui) {
    let due = ui.tour.as_mut().is_some_and(|tour| {
        let due = tour.raise_at.is_some_and(|at| Instant::now() >= at);
        if due {
            tour.raise_at = None;
        }
        due
    });
    if due && let Some(window) = &ui.window {
        crate::host::bring_window_forward(window);
    }
}

#[cfg(not(target_os = "macos"))]
fn raise_after_shortcut(_: &mut Ui) {}

/// Ends the tour from the window closing or Escape.
pub fn dismiss(ui: &mut Ui) {
    input(ui, Input::Dismiss);
}

fn remember_step(ui: &Ui, before: Step) {
    if !ui.fixture
        && let Some(tour) = &ui.tour
        && !tour.flow.manual()
        && !tour.completed
        && tour.flow.step() != before
    {
        flow::record_step(tour.flow.step());
    }
}

fn apply(ui: &mut Ui, effect: Effect) {
    match effect {
        Effect::None => {}
        Effect::OpenAccessibility => ui.open_accessibility(),
        Effect::ToggleLaunchAtLogin => {
            if ui.fixture {
                if let Some(agent) = &mut ui.agent {
                    agent.launch_at_login = match agent.launch_at_login {
                        UiLaunchAtLogin::On | UiLaunchAtLogin::NeedsApproval => {
                            UiLaunchAtLogin::Off
                        }
                        UiLaunchAtLogin::Off | UiLaunchAtLogin::Failed(_) => UiLaunchAtLogin::On,
                    };
                }
                let status = ui_setup_state(ui);
                if let Some(tour) = &mut ui.tour {
                    tour.flow.on(Input::Status(status));
                }
            } else {
                ui.toggle_launch_at_login();
            }
        }
        Effect::AdvanceSoon => {
            if let Some(tour) = &mut ui.tour {
                tour.advance_at = Some(Instant::now());
            }
        }
        Effect::Finish => finish(ui),
    }
}

/// Ends the tour. A first-run tour releases the hold it placed on automatic
/// tiling, so tiling starts only now rather than behind the setup window.
fn finish(ui: &mut Ui) {
    let Some(tour) = ui.tour.take() else {
        return;
    };
    if !ui.fixture {
        flow::record_completion();
    }
    if !tour.flow.manual() {
        ui.finish_setup();
    }
}

fn ui_setup_state(ui: &Ui) -> SetupState {
    let accessibility = ui.agent.as_ref().is_some_and(|agent| agent.accessibility);
    SetupState {
        accessibility,
        launch_at_login: ui_launch_at_login(ui),
    }
}

fn ui_launch_at_login(ui: &Ui) -> bool {
    ui.agent.as_ref().is_some_and(|agent| {
        matches!(
            agent.launch_at_login,
            UiLaunchAtLogin::On | UiLaunchAtLogin::NeedsApproval
        )
    })
}

fn tour_scene(ui: &mut Ui, step: Step, accessibility: bool, asked: bool) {
    reset_fixture(ui, Pane::Rules);
    ui.tour = Some(Tour::fixture(step, accessibility, asked, Vec::new(), ui));
}

pub(crate) const SCENES: &[Scene] = &[
    ("onboarding-welcome", |ui| {
        tour_scene(ui, Step::Welcome, false, false)
    }),
    ("onboarding-conflict", |ui| {
        reset_fixture(ui, Pane::Rules);
        ui.tour = Some(Tour::fixture(
            Step::Welcome,
            false,
            false,
            vec!["AeroSpace".into()],
            ui,
        ));
    }),
    ("onboarding-setup", |ui| {
        tour_scene(ui, Step::Setup, false, false)
    }),
    ("onboarding-setup-waiting", |ui| {
        tour_scene(ui, Step::Setup, false, true)
    }),
    ("onboarding-setup-ready", |ui| {
        tour_scene(ui, Step::Setup, true, false)
    }),
    ("onboarding-done", |ui| {
        tour_scene(ui, Step::Done, true, false)
    }),
    // Focus moved to the right-hand window; flipping the split is next.
    ("onboarding-sandbox-step2", |ui| {
        practice_scene(ui, &[Key::Right])
    }),
    // Split flipped to stacked, then over on workspace 2 with its own three windows.
    ("onboarding-sandbox-workspace-2", |ui| {
        practice_scene(ui, &[Key::Right, Key::Flip, Key::Workspace(2)])
    }),
    ("onboarding-sandbox-complete", |ui| {
        practice_scene(
            ui,
            &[Key::Right, Key::Flip, Key::Workspace(2), Key::Workspace(1)],
        )
    }),
];

fn practice_scene(ui: &mut Ui, keys: &[Key]) {
    tour_scene(ui, Step::Done, true, false);
    if let Some(tour) = &mut ui.tour {
        for key in keys {
            tour.sandbox.press(*key);
        }
        // Scenes open at rest, not mid-press.
        tour.sandbox.last = None;
    }
}
