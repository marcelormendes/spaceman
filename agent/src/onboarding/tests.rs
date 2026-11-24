use std::sync::atomic::{AtomicUsize, Ordering};

use super::flow::{
    Effect, Flow, Input, SetupState, Step, completion_recorded_at, launch_step_at,
    record_completion_at, record_step_at, saved_step_at,
};
use super::sandbox::{Key, Lesson, Sandbox, Split};
use crate::{apps::window_manager_name, controls::PermissionControl};

fn temp_dir(name: &str) -> std::path::PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "spaceman-onboarding-test-{name}-{}-{id}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn fresh_untrusted_launch_opens_welcome_and_blocks_continue_until_granted() {
    let dir = temp_dir("fresh");
    assert_eq!(launch_step_at(&dir, false), Some(Step::Welcome));

    let mut flow = Flow::new(
        Step::Welcome,
        SetupState {
            accessibility: false,
            launch_at_login: false,
        },
        false,
    );
    assert_eq!(flow.view().step, Step::Welcome);
    assert_eq!(flow.view().count, 3);
    assert!(!flow.view().can_go_back);

    assert_eq!(flow.on(Input::Continue), Effect::None);
    assert_eq!(flow.view().step, Step::Setup);
    assert_eq!(flow.view().direction, 1);
    assert_eq!(flow.view().index, 1);
    assert_eq!(flow.view().accessibility, PermissionControl::Allow);
    assert!(!flow.view().ready);

    // Continue stays blocked while Accessibility is missing.
    assert_eq!(flow.on(Input::Continue), Effect::None);
    assert_eq!(flow.view().step, Step::Setup);

    // Asking opens the Accessibility guide and enters Waiting.
    assert_eq!(
        flow.on(Input::GrantAccessibility),
        Effect::OpenAccessibility
    );
    assert_eq!(flow.view().accessibility, PermissionControl::Waiting);

    // Granting triggers AdvanceSoon so the green check settles before advancing.
    assert_eq!(
        flow.on(Input::Status(SetupState {
            accessibility: true,
            launch_at_login: true,
        })),
        Effect::AdvanceSoon
    );
    assert_eq!(
        flow.view().accessibility,
        PermissionControl::Allowed { animated: true }
    );
    assert!(flow.view().ready);

    // The tour is three steps: Welcome, Setup, Done. Choosing app rules lives in
    // Settings > Rules rather than in the first-run tour.
    assert_eq!(flow.on(Input::Continue), Effect::None);
    assert_eq!(flow.view().step, Step::Done);
    assert_eq!(flow.view().index, 2);

    assert_eq!(flow.on(Input::Continue), Effect::Finish);
    drop(std::fs::remove_dir_all(dir));
}

#[test]
fn existing_trusted_install_records_completion_without_opening_window() {
    let dir = temp_dir("trusted-upgrade");
    assert_eq!(launch_step_at(&dir, true), None);
    assert!(completion_recorded_at(&dir));
    // Subsequent launches even without Accessibility do not restart Welcome.
    assert_eq!(launch_step_at(&dir, false), None);
    drop(std::fs::remove_dir_all(dir));
}

#[test]
fn saved_setup_step_resumes_at_done_after_accessibility_grant_relaunch() {
    let dir = temp_dir("resume");
    record_step_at(&dir, Step::Setup).unwrap();
    assert_eq!(saved_step_at(&dir), Some(Step::Setup));
    assert_eq!(launch_step_at(&dir, true), Some(Step::Setup));

    let flow = Flow::new(
        Step::Setup,
        SetupState {
            accessibility: true,
            launch_at_login: false,
        },
        false,
    );
    assert_eq!(flow.step(), Step::Done);

    record_completion_at(&dir).unwrap();
    assert!(completion_recorded_at(&dir));
    assert_eq!(saved_step_at(&dir), None);
    drop(std::fs::remove_dir_all(dir));
}

#[test]
fn an_in_progress_tour_saved_on_the_removed_rules_step_resumes_at_done() {
    let dir = temp_dir("rules-migration");
    std::fs::write(dir.join("onboarding-step"), "rules").unwrap();
    assert_eq!(saved_step_at(&dir), Some(Step::Done));
    assert_eq!(launch_step_at(&dir, true), Some(Step::Done));
    drop(std::fs::remove_dir_all(dir));
}

#[test]
fn first_run_tour_can_be_dismissed_and_only_a_manual_one_is_complete() {
    let mut first_run = Flow::new(
        Step::Welcome,
        SetupState {
            accessibility: false,
            launch_at_login: false,
        },
        false,
    );
    assert!(!first_run.view().manual);
    assert_eq!(first_run.on(Input::Dismiss), Effect::Finish);

    let manual = Flow::new(
        Step::Welcome,
        SetupState {
            accessibility: true,
            launch_at_login: true,
        },
        true,
    );
    assert!(manual.view().manual);
}

#[test]
fn manual_tour_walks_all_steps_and_allows_dismiss() {
    let mut flow = Flow::new(
        Step::Welcome,
        SetupState {
            accessibility: true,
            launch_at_login: true,
        },
        true,
    );
    assert_eq!(flow.on(Input::Continue), Effect::None);
    assert_eq!(flow.step(), Step::Setup);
    assert!(flow.view().ready);
    assert_eq!(flow.on(Input::Continue), Effect::None);
    assert_eq!(flow.step(), Step::Done);
    assert_eq!(flow.on(Input::Back), Effect::None);
    assert_eq!(flow.step(), Step::Setup);
    assert_eq!(flow.view().direction, -1);
    assert_eq!(flow.on(Input::Dismiss), Effect::Finish);
}

#[test]
fn recognizes_conflicting_window_managers_by_bundle_or_daemon_name() {
    assert_eq!(
        window_manager_name(Some("bobko.aerospace"), None),
        Some("AeroSpace")
    );
    assert_eq!(
        window_manager_name(Some("com.knollsoft.Rectangle"), None),
        Some("Rectangle")
    );
    assert_eq!(window_manager_name(None, Some("yabai")), Some("yabai"));
    assert_eq!(
        window_manager_name(Some("com.apple.Safari"), Some("Safari")),
        None
    );
}

#[test]
fn practice_keys_map_from_ansi_key_codes() {
    assert_eq!(Key::from_key_code(4), Some(Key::Left));
    assert_eq!(Key::from_key_code(38), Some(Key::Down));
    assert_eq!(Key::from_key_code(40), Some(Key::Up));
    assert_eq!(Key::from_key_code(37), Some(Key::Right));
    assert_eq!(Key::from_key_code(44), Some(Key::Flip));
    assert_eq!(Key::from_key_code(18), Some(Key::Workspace(1)));
    assert_eq!(Key::from_key_code(19), Some(Key::Workspace(2)));
    assert_eq!(Key::from_key_code(20), None);
}

#[test]
fn moving_focus_counts_only_when_a_tile_lies_that_way() {
    let mut sandbox = Sandbox::default();
    assert_eq!(sandbox.current(), Some(Lesson::Focus));
    // Side by side: nothing is below or to the left of the first tile.
    sandbox.press(Key::Down);
    sandbox.press(Key::Left);
    assert!(!sandbox.done(Lesson::Focus));
    assert_eq!(sandbox.focus(1), 0);
    assert_eq!(sandbox.presses, 2);

    sandbox.press(Key::Right);
    assert_eq!(sandbox.focus(1), 1);
    assert!(sandbox.done(Lesson::Focus));
    assert_eq!(sandbox.current(), Some(Lesson::Flip));
    sandbox.press(Key::Left);
    assert_eq!(sandbox.focus(1), 0);
}

#[test]
fn flipping_stacks_the_split_and_swaps_the_focus_keys() {
    let mut sandbox = Sandbox::default();
    assert_eq!(sandbox.focus_keys(), [Key::Left, Key::Right]);
    sandbox.press(Key::Flip);
    assert_eq!(sandbox.split(1), Split::Stacked);
    assert!(sandbox.done(Lesson::Flip));
    assert_eq!(
        sandbox.frames(1),
        vec![[0.0, 0.0, 1.0, 0.5], [0.0, 0.5, 1.0, 0.5]]
    );
    assert_eq!(sandbox.focus_keys(), [Key::Up, Key::Down]);
    sandbox.press(Key::Down);
    assert_eq!(sandbox.focus(1), 1);
    sandbox.press(Key::Flip);
    assert_eq!(sandbox.split(1), Split::Side);
    // Focus stays on the same window through a flip.
    assert_eq!(sandbox.focus(1), 1);
}

#[test]
fn switching_counts_once_there_and_back_and_each_desk_keeps_its_own_layout() {
    let mut sandbox = Sandbox::default();
    sandbox.press(Key::Workspace(1));
    assert_eq!(sandbox.switches, 0);

    sandbox.press(Key::Workspace(2));
    assert_eq!(sandbox.workspace, 2);
    assert_eq!(sandbox.switches, 1);
    assert!(!sandbox.done(Lesson::Switch));
    // Workspace 2 holds three windows: one on the left, two stacked on the right.
    assert_eq!(sandbox.frames(2).len(), 3);
    sandbox.press(Key::Right);
    assert_eq!(sandbox.focus(2), 1);
    sandbox.press(Key::Down);
    assert_eq!(sandbox.focus(2), 2);
    sandbox.press(Key::Left);
    assert_eq!(sandbox.focus(2), 0);
    sandbox.press(Key::Flip);
    assert_eq!(sandbox.split(2), Split::Stacked);
    assert_eq!(sandbox.split(1), Split::Side);

    sandbox.press(Key::Workspace(1));
    assert_eq!(sandbox.workspace, 1);
    assert!(sandbox.done(Lesson::Switch));
    assert!(sandbox.complete());
    assert_eq!(sandbox.current(), None);
}

#[test]
fn practice_presses_play_only_on_the_done_step() {
    let mut ui = crate::playground::blank_fixture();
    assert!(!super::press(&mut ui, Key::Right));

    super::tour_scene(&mut ui, Step::Welcome, true, false);
    assert!(!super::press(&mut ui, Key::Right));
    assert_eq!(ui.tour.as_ref().map(|tour| tour.sandbox.presses), Some(0));

    super::tour_scene(&mut ui, Step::Done, true, false);
    assert!(super::press(&mut ui, Key::Right));
    let tour = ui.tour.as_ref().unwrap();
    assert_eq!(tour.sandbox.focus(1), 1);
    // Practising never ends the tour; Done stays the only way out.
    assert_eq!(tour.flow.step(), Step::Done);
}
