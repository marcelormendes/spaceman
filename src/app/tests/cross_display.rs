//! Moving windows to a workspace on another display (Option+Shift+number).
use super::*;

/// Where a tiled window on Main's only tile lands.
fn main_tile() -> crate::Rect {
    outputs().main().frame
}

fn at(x: i32) -> crate::Rect {
    crate::Rect { x, ..frame(0) }
}

#[test]
fn moving_a_window_to_the_workspace_on_screen_on_another_display_tiles_it_there() {
    let window = window(20);
    let mut backend = FakeBackend {
        focused: Some(window.clone()),
        ..FakeBackend::default()
    };
    backend.frames.insert(window.clone(), frame(1050));
    let mut runtime = runtime(backend);
    bind(&mut runtime, WorkspaceId::SIX, &window);
    runtime.record_focused_workspace(WorkspaceId::SIX);

    runtime
        .move_focused(WorkspaceId::ONE, outputs(), &AtomicBool::new(false))
        .unwrap();

    assert_eq!(
        runtime.workspaces.workspace_of(&window),
        Some(WorkspaceId::ONE)
    );
    assert_eq!(runtime.backend.frames[&window], main_tile());
    assert!(!runtime.parked.contains_key(&window));
    assert_eq!(runtime.backend.focused, Some(window));
    // Focus followed the window to Main, so Option+Tab goes back to Secondary.
    assert_eq!(runtime.last_focused_workspace, Some(WorkspaceId::ONE));
    assert_eq!(runtime.previous_focused_workspace, Some(WorkspaceId::SIX));
}

#[test]
fn moving_a_window_to_a_hidden_workspace_on_another_display_parks_it_where_it_is() {
    let window = window(20);
    let mut backend = FakeBackend {
        focused: Some(window.clone()),
        // macOS clamps a write off Main's edge back onto Secondary, where
        // the window is. Moving it used to fail outright.
        refuses_position: Some(|target| target.x < 0),
        ..FakeBackend::default()
    };
    backend.frames.insert(window.clone(), frame(1050));
    let mut runtime = runtime(backend);
    bind(&mut runtime, WorkspaceId::SIX, &window);
    let target = WorkspaceId::new(2).unwrap();
    let cancelled = AtomicBool::new(false);

    runtime.move_focused(target, outputs(), &cancelled).unwrap();

    assert_eq!(runtime.workspaces.workspace_of(&window), Some(target));
    let displays = outputs().frames().collect::<Vec<_>>();
    let secondary = outputs().displays()[1].frame;
    assert_eq!(
        runtime.backend.frames[&window],
        crate::geometry::parked_frame(secondary, &displays, frame(1050))
    );
    // Recovery brings it back on Main, where its workspace lives.
    assert_eq!(runtime.parked[&window], frame(50));
    assert_eq!(runtime.backend.persisted[&window], frame(50));

    runtime.switch(target, outputs(), &cancelled).unwrap();

    assert_eq!(runtime.backend.frames[&window], main_tile());
    assert!(!runtime.parked.contains_key(&window));
    assert!(runtime.backend.persisted.is_empty());
}

#[test]
fn a_cross_display_move_that_cannot_park_leaves_the_window_where_it_was() {
    let window = window(20);
    let mut backend = FakeBackend {
        focused: Some(window.clone()),
        refuses_position: Some(|target| target.x != 1050),
        ..FakeBackend::default()
    };
    backend.frames.insert(window.clone(), frame(1050));
    let mut runtime = runtime(backend);
    bind(&mut runtime, WorkspaceId::SIX, &window);

    let Err(error) = runtime.move_focused(
        WorkspaceId::new(2).unwrap(),
        outputs(),
        &AtomicBool::new(false),
    ) else {
        panic!("a window that could not be parked was moved");
    };

    assert_eq!(error, "Window could not be parked in the target workspace");
    assert_eq!(
        runtime.workspaces.workspace_of(&window),
        Some(WorkspaceId::SIX)
    );
    assert_eq!(runtime.backend.frames[&window], frame(1050));
    assert!(runtime.parked.is_empty());
    assert!(runtime.backend.persisted.is_empty());
}

#[test]
fn moving_a_floating_window_to_the_workspace_on_screen_on_another_display_carries_it_there() {
    let floating = window(20);
    let mut backend = FakeBackend {
        focused: Some(floating.clone()),
        ..FakeBackend::default()
    };
    backend.frames.insert(floating.clone(), at(1100));
    let mut runtime = runtime(backend);
    runtime
        .workspaces
        .own_floating(floating.clone(), WorkspaceId::SIX);

    let status = runtime
        .move_focused(WorkspaceId::ONE, outputs(), &AtomicBool::new(false))
        .unwrap();

    assert_eq!(status.failed, 0);
    assert_eq!(
        runtime.workspaces.floating_workspace_of(&floating),
        Some(WorkspaceId::ONE)
    );
    // Same size, same place relative to the display, now on Main.
    assert_eq!(runtime.backend.frames[&floating], at(100));
}

#[test]
fn a_floating_window_moved_to_a_hidden_workspace_on_another_display_shows_up_there() {
    let floating = window(20);
    let mut backend = FakeBackend {
        focused: Some(floating.clone()),
        ..FakeBackend::default()
    };
    backend.frames.insert(floating.clone(), at(1100));
    let mut runtime = runtime(backend);
    runtime
        .workspaces
        .own_floating(floating.clone(), WorkspaceId::SIX);
    let target = WorkspaceId::new(2).unwrap();
    let cancelled = AtomicBool::new(false);

    runtime.move_focused(target, outputs(), &cancelled).unwrap();
    assert!(runtime.parked.contains_key(&floating));

    runtime.switch(target, outputs(), &cancelled).unwrap();

    assert_eq!(runtime.backend.frames[&floating], at(100));
    assert!(!runtime.parked.contains_key(&floating));
}

#[test]
fn reassigning_workspace_display_via_config_relocates_recovery_frames_and_parks_safely() {
    let cancelled = AtomicBool::new(false);
    let seven = WorkspaceId::new(7).unwrap();
    let to_main =
        Config::parse("version = 1\n[[workspaces]]\nworkspace = 6\nmonitor = 'main'").unwrap();

    // Part A: hidden parked windows on workspace 6 (Secondary) have their recovery frames
    // translated onto Main when config reassigns workspace 6 to Main.
    let settings = window(10);
    let notes = crate::PhysicalWindowId {
        app: AppInstanceId {
            pid: 2,
            launched_at: 2,
        },
        window_id: 20,
    };
    let secondary_displays: Vec<_> = outputs().frames().collect();
    let secondary_frame = outputs().secondary().unwrap().frame;
    let settings_orig = frame(1050);
    let notes_orig = frame(1100);
    let mut backend = FakeBackend {
        observation: Some(crate::NativeObservation {
            cycle: 0,
            mouse_down: false,
            applications: Vec::new(),
        }),
        ..FakeBackend::default()
    };
    backend.frames.insert(
        settings.clone(),
        crate::geometry::parked_frame(secondary_frame, &secondary_displays, settings_orig),
    );
    backend.frames.insert(
        notes.clone(),
        crate::geometry::parked_frame(secondary_frame, &secondary_displays, notes_orig),
    );
    let mut runtime = runtime_with_parked(
        backend,
        [
            (settings.clone(), settings_orig),
            (notes.clone(), notes_orig),
        ],
    );
    runtime.workspaces.switch(seven);
    runtime
        .workspaces
        .own_floating(settings.clone(), WorkspaceId::SIX);
    bind(&mut runtime, WorkspaceId::SIX, &notes);

    runtime.replace_config(to_main.clone());
    let status = runtime.cycle(outputs(), &cancelled).unwrap();

    assert!(
        outputs()
            .main()
            .frame
            .contains_center(runtime.parked[&settings])
    );
    assert!(
        outputs()
            .main()
            .frame
            .contains_center(runtime.parked[&notes])
    );
    let main_menu: Vec<_> = status.menu[0]
        .workspaces
        .iter()
        .map(|entry| entry.workspace)
        .collect();
    let secondary_menu: Vec<_> = status.menu[1]
        .workspaces
        .iter()
        .map(|entry| entry.workspace)
        .collect();
    assert_eq!(main_menu, vec![WorkspaceId::ONE, WorkspaceId::SIX]);
    assert_eq!(secondary_menu, vec![seven]);

    runtime
        .switch(WorkspaceId::SIX, outputs(), &cancelled)
        .unwrap();
    assert!(
        outputs()
            .main()
            .frame
            .contains_center(runtime.backend.frames[&settings])
    );
    assert!(
        outputs()
            .main()
            .frame
            .contains_center(runtime.backend.frames[&notes])
    );

    // Part B: visible workspace 6 on Secondary moved to Main (becoming hidden on Main) parks
    // its window off Secondary's edge (even if Main-edge writes are clamped) and translates its
    // recovery frame to Main.
    let visible_win = window(30);
    let mut backend = FakeBackend {
        observation: Some(observation(&visible_win, frame(1050))),
        refuses_position: Some(|target| target.x < 0),
        ..FakeBackend::default()
    };
    backend.frames.insert(visible_win.clone(), frame(1050));
    let mut runtime = super::runtime(backend);
    bind(&mut runtime, WorkspaceId::SIX, &visible_win);

    runtime.replace_config(to_main);
    let status = runtime.cycle(outputs(), &cancelled).unwrap();

    assert_eq!(status.failed, 0);
    assert!(
        outputs()
            .main()
            .frame
            .contains_center(runtime.parked[&visible_win])
    );
    let main_menu: Vec<_> = status.menu[0]
        .workspaces
        .iter()
        .map(|entry| entry.workspace)
        .collect();
    assert_eq!(main_menu, vec![WorkspaceId::ONE, WorkspaceId::SIX]);
}
