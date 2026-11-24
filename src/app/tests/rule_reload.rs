use super::*;

fn window_for_app(pid: i32, window_id: u32) -> crate::PhysicalWindowId {
    crate::PhysicalWindowId {
        app: AppInstanceId {
            pid,
            launched_at: pid as u64,
        },
        window_id,
    }
}

fn multi_app_observation(
    apps: &[(&str, &[(crate::PhysicalWindowId, crate::Rect)])],
) -> crate::NativeObservation {
    crate::NativeObservation {
        cycle: 0,
        mouse_down: false,
        applications: apps
            .iter()
            .map(|(bundle_id, windows)| crate::AppObservation::Complete {
                app: crate::ObservedApplication {
                    id: windows[0].0.app.clone(),
                    bundle_id: (*bundle_id).into(),
                },
                focused: None,
                windows: windows
                    .iter()
                    .map(|(id, frame)| crate::ObservedWindow {
                        id: id.clone(),
                        frame: *frame,
                        resizable: true,
                        title: None,
                    })
                    .collect(),
            })
            .collect(),
    }
}

#[test]
fn adding_or_changing_route_rule_moves_open_windows_live_and_populates_target_workspace() {
    let keep = window(10);
    let moved_a = window_for_app(2, 20);
    let moved_b = window_for_app(2, 21);
    let cancelled = AtomicBool::new(false);
    let mut backend = FakeBackend {
        focused: Some(keep.clone()),
        frontmost: Some(keep.app.clone()),
        observation: Some(multi_app_observation(&[
            ("com.example.Keep", &[(keep.clone(), frame(50))]),
            (
                "com.example.Moved",
                &[(moved_a.clone(), frame(300)), (moved_b.clone(), frame(600))],
            ),
        ])),
        ..FakeBackend::default()
    };
    backend.frames.insert(keep.clone(), frame(50));
    backend.frames.insert(moved_a.clone(), frame(300));
    backend.frames.insert(moved_b.clone(), frame(600));
    let mut runtime = runtime(backend);

    // Initially all three open windows are tiled on workspace 1.
    runtime.cycle(outputs(), &cancelled).unwrap();
    assert_eq!(
        runtime.workspaces.workspace_of(&keep),
        Some(WorkspaceId::ONE)
    );
    assert_eq!(
        runtime.workspaces.workspace_of(&moved_a),
        Some(WorkspaceId::ONE)
    );
    assert_eq!(
        runtime.workspaces.workspace_of(&moved_b),
        Some(WorkspaceId::ONE)
    );

    // Adding a route rule to workspace 2 moves both open windows immediately,
    // reflows workspace 1, and populates workspace 2 in the workspace menu.
    let to_two = Config::parse(
        "version = 1\n[[rules]]\nbundle_id = 'com.example.Moved'\naction = 'route'\nworkspace = 2",
    )
    .unwrap();
    runtime.replace_config(to_two);
    let status = runtime.cycle(outputs(), &cancelled).unwrap();

    assert_eq!(
        runtime.workspaces.workspace_of(&keep),
        Some(WorkspaceId::ONE)
    );
    assert_eq!(
        runtime.workspaces.workspace_of(&moved_a),
        Some(WorkspaceId::TWO)
    );
    assert_eq!(
        runtime.workspaces.workspace_of(&moved_b),
        Some(WorkspaceId::TWO)
    );
    assert!(runtime.parked.contains_key(&moved_a));
    assert!(runtime.parked.contains_key(&moved_b));
    assert_eq!(runtime.backend.frames[&keep], outputs().main().frame);
    let listed: Vec<_> = status.menu[0]
        .workspaces
        .iter()
        .map(|entry| entry.workspace)
        .collect();
    assert_eq!(listed, vec![WorkspaceId::ONE, WorkspaceId::TWO]);

    // Changing the rule from workspace 2 to workspace 4 moves the already-parked
    // windows from workspace 2 to workspace 4 live.
    let to_four = Config::parse(
        "version = 1\n[[rules]]\nbundle_id = 'com.example.Moved'\naction = 'route'\nworkspace = 4",
    )
    .unwrap();
    runtime.replace_config(to_four);
    let status = runtime.cycle(outputs(), &cancelled).unwrap();

    assert_eq!(
        runtime.workspaces.workspace_of(&moved_a),
        Some(WorkspaceId::FOUR)
    );
    assert_eq!(
        runtime.workspaces.workspace_of(&moved_b),
        Some(WorkspaceId::FOUR)
    );
    assert!(runtime.workspaces.windows(WorkspaceId::TWO).is_empty());
    let listed: Vec<_> = status.menu[0]
        .workspaces
        .iter()
        .map(|entry| entry.workspace)
        .collect();
    assert_eq!(listed, vec![WorkspaceId::ONE, WorkspaceId::FOUR]);

    // Changing the rule back to the currently visible workspace 1 unparks and
    // tiles both windows immediately.
    let to_one = Config::parse(
        "version = 1\n[[rules]]\nbundle_id = 'com.example.Moved'\naction = 'route'\nworkspace = 1",
    )
    .unwrap();
    runtime.replace_config(to_one);
    runtime.cycle(outputs(), &cancelled).unwrap();

    assert_eq!(
        runtime.workspaces.workspace_of(&moved_a),
        Some(WorkspaceId::ONE)
    );
    assert_eq!(
        runtime.workspaces.workspace_of(&moved_b),
        Some(WorkspaceId::ONE)
    );
    assert!(!runtime.parked.contains_key(&moved_a));
    assert!(!runtime.parked.contains_key(&moved_b));
    assert!(
        outputs()
            .main()
            .frame
            .contains_center(runtime.backend.frames[&moved_a])
    );
    assert!(
        outputs()
            .main()
            .frame
            .contains_center(runtime.backend.frames[&moved_b])
    );
}

#[test]
fn changing_route_rule_to_named_workspace_enables_it_and_moves_open_window() {
    let chat = window(10);
    let cancelled = AtomicBool::new(false);
    let mut backend = FakeBackend {
        observation: Some(observation_for("com.example.Chat", &chat, frame(50))),
        ..FakeBackend::default()
    };
    backend.frames.insert(chat.clone(), frame(50));
    let mut runtime = runtime(backend);

    runtime.cycle(outputs(), &cancelled).unwrap();
    let named = WorkspaceId::named('M').unwrap();
    assert!(!runtime.workspaces.is_enabled(named));

    let config = Config::parse(
        "version = 1\n[[rules]]\nbundle_id = 'com.example.Chat'\naction = 'route'\nworkspace = 'M'",
    )
    .unwrap();
    runtime.replace_config(config);
    let status = runtime.cycle(outputs(), &cancelled).unwrap();

    assert!(runtime.workspaces.is_enabled(named));
    assert_eq!(runtime.workspaces.workspace_of(&chat), Some(named));
    assert!(runtime.parked.contains_key(&chat));
    assert!(
        status.menu[0]
            .workspaces
            .iter()
            .any(|entry| entry.workspace == named)
    );
}

#[test]
fn removing_or_rerouting_float_rule_applies_live_on_visible_and_hidden_workspaces() {
    let win = window(10);
    let cancelled = AtomicBool::new(false);
    let mut backend = FakeBackend {
        observation: Some(observation(&win, frame(50))),
        ..FakeBackend::default()
    };
    backend.frames.insert(win.clone(), frame(50));
    let float_config =
        Config::parse("version = 1\n[[rules]]\nbundle_id = 'com.example.Test'\naction = 'float'")
            .unwrap();
    let mut runtime = runtime_with_config(backend, float_config);

    runtime.cycle(outputs(), &cancelled).unwrap();
    assert_eq!(
        runtime.workspaces.floating_workspace_of(&win),
        Some(WorkspaceId::ONE)
    );
    // Simulate a restart where restored_floating recorded the window in manual_floating.
    runtime.manual_floating.insert(win.clone());

    // Removing the float rule on a visible workspace re-tiles immediately.
    runtime.replace_config(Config::default());
    runtime.cycle(outputs(), &cancelled).unwrap();
    assert_eq!(
        runtime.workspaces.workspace_of(&win),
        Some(WorkspaceId::ONE)
    );
    assert_eq!(runtime.workspaces.floating_workspace_of(&win), None);
    assert!(!runtime.manual_floating.contains(&win));

    // Re-adding float, switching to workspace 2 (so workspace 1 is hidden),
    // and removing the float rule converts the hidden floating window to tiled too.
    let float_again =
        Config::parse("version = 1\n[[rules]]\nbundle_id = 'com.example.Test'\naction = 'float'")
            .unwrap();
    runtime.replace_config(float_again);
    runtime.cycle(outputs(), &cancelled).unwrap();
    runtime
        .switch(WorkspaceId::TWO, outputs(), &cancelled)
        .unwrap();
    assert_eq!(
        runtime.workspaces.floating_workspace_of(&win),
        Some(WorkspaceId::ONE)
    );

    runtime.replace_config(Config::default());
    runtime.cycle(outputs(), &cancelled).unwrap();
    assert_eq!(
        runtime.workspaces.workspace_of(&win),
        Some(WorkspaceId::ONE)
    );
    assert_eq!(runtime.workspaces.floating_workspace_of(&win), None);

    // Dragging it to visible workspace 6 tiles it on the secondary display.
    let route_six = Config::parse(
        "version = 1\n[[rules]]\nbundle_id = 'com.example.Test'\naction = 'route'\nworkspace = 6",
    )
    .unwrap();
    runtime.replace_config(route_six);
    runtime.cycle(outputs(), &cancelled).unwrap();
    assert_eq!(
        runtime.workspaces.workspace_of(&win),
        Some(WorkspaceId::SIX)
    );
    assert!(runtime.backend.frames[&win].x >= 1000);
}

#[test]
fn floating_hidden_parked_tile_preserves_recovery_frame() {
    let win = window(10);
    let cancelled = AtomicBool::new(false);
    let mut backend = FakeBackend {
        observation: Some(observation(&win, frame(50))),
        ..FakeBackend::default()
    };
    backend.frames.insert(win.clone(), frame(50));
    let route_two = Config::parse(
        "version = 1\n[[rules]]\nbundle_id = 'com.example.Test'\naction = 'route'\nworkspace = 2",
    )
    .unwrap();
    let mut runtime = runtime_with_config(backend, route_two);

    runtime.cycle(outputs(), &cancelled).unwrap();
    assert_eq!(
        runtime.workspaces.workspace_of(&win),
        Some(WorkspaceId::TWO)
    );
    let recovery_before = runtime.parked[&win];

    let to_float =
        Config::parse("version = 1\n[[rules]]\nbundle_id = 'com.example.Test'\naction = 'float'")
            .unwrap();
    runtime.replace_config(to_float);
    runtime.cycle(outputs(), &cancelled).unwrap();

    assert_eq!(runtime.workspaces.workspace_of(&win), None);
    assert_eq!(
        runtime.workspaces.floating_workspace_of(&win),
        Some(WorkspaceId::TWO)
    );
    assert_eq!(runtime.parked.get(&win).copied(), Some(recovery_before));
}

#[test]
fn non_resizable_window_stays_floating_on_current_workspace_when_route_rule_added() {
    let dialog = window(10);
    let cancelled = AtomicBool::new(false);
    let mut observed = observation(&dialog, frame(100));
    if let crate::AppObservation::Complete { windows, .. } = &mut observed.applications[0] {
        windows[0].resizable = false;
    }
    let mut backend = FakeBackend {
        observation: Some(observed),
        ..FakeBackend::default()
    };
    backend.frames.insert(dialog.clone(), frame(100));
    let mut runtime = runtime(backend);

    runtime.cycle(outputs(), &cancelled).unwrap();
    assert_eq!(
        runtime.workspaces.floating_workspace_of(&dialog),
        Some(WorkspaceId::ONE)
    );

    let route_two = Config::parse(
        "version = 1\n[[rules]]\nbundle_id = 'com.example.Test'\naction = 'route'\nworkspace = 2",
    )
    .unwrap();
    runtime.replace_config(route_two);
    runtime.cycle(outputs(), &cancelled).unwrap();

    assert_eq!(
        runtime.workspaces.floating_workspace_of(&dialog),
        Some(WorkspaceId::ONE)
    );
    assert!(!runtime.parked.contains_key(&dialog));
}

#[test]
fn unrelated_rule_reload_preserves_manually_moved_windows() {
    let first = window(10);
    let other = window_for_app(2, 20);
    let cancelled = AtomicBool::new(false);
    let mut backend = FakeBackend {
        observation: Some(multi_app_observation(&[
            ("com.example.First", &[(first.clone(), frame(50))]),
            ("com.example.Other", &[(other.clone(), frame(400))]),
        ])),
        ..FakeBackend::default()
    };
    backend.frames.insert(first.clone(), frame(50));
    backend.frames.insert(other.clone(), frame(400));
    let initial = Config::parse(
        "version = 1\n[[rules]]\nbundle_id = 'com.example.First'\naction = 'route'\nworkspace = 2",
    )
    .unwrap();
    let mut runtime = runtime_with_config(backend, initial);
    // Suppose the user previously moved `first` by hand to workspace 3.
    bind(&mut runtime, WorkspaceId::THREE, &first);
    runtime
        .memory
        .bundles
        .insert(first.app.clone(), "com.example.First".into());
    runtime.cycle(outputs(), &cancelled).unwrap();

    let updated = Config::parse(
        "version = 1
[[rules]]
bundle_id = 'com.example.First'
action = 'route'
workspace = 2
[[rules]]
bundle_id = 'com.example.Other'
action = 'route'
workspace = 4",
    )
    .unwrap();
    runtime.replace_config(updated);
    runtime.cycle(outputs(), &cancelled).unwrap();

    assert_eq!(
        runtime.workspaces.workspace_of(&first),
        Some(WorkspaceId::THREE)
    );
    assert_eq!(
        runtime.workspaces.workspace_of(&other),
        Some(WorkspaceId::FOUR)
    );
}

#[test]
fn ignoring_a_hidden_parked_window_restores_and_releases_it() {
    let win = window(10);
    let cancelled = AtomicBool::new(false);
    let mut backend = FakeBackend {
        observation: Some(observation(&win, frame(50))),
        ..FakeBackend::default()
    };
    backend.frames.insert(win.clone(), frame(50));
    let route_two = Config::parse(
        "version = 1\n[[rules]]\nbundle_id = 'com.example.Test'\naction = 'route'\nworkspace = 2",
    )
    .unwrap();
    let mut runtime = runtime_with_config(backend, route_two);

    runtime.cycle(outputs(), &cancelled).unwrap();
    assert!(runtime.parked.contains_key(&win));

    let ignore =
        Config::parse("version = 1\n[[rules]]\nbundle_id = 'com.example.Test'\naction = 'ignore'")
            .unwrap();
    runtime.replace_config(ignore);
    runtime.cycle(outputs(), &cancelled).unwrap();

    assert_eq!(runtime.workspaces.workspace_of_any(&win), None);
    assert!(!runtime.parked.contains_key(&win));
    assert!(
        outputs()
            .main()
            .frame
            .contains_center(runtime.backend.frames[&win])
    );
}

#[test]
fn rule_reload_retries_only_failed_or_unavailable_windows() {
    let win = window(10);
    let other = window_for_app(2, 20);
    let cancelled = AtomicBool::new(false);
    let mut backend = FakeBackend {
        observation: Some(multi_app_observation(&[
            ("com.example.Test", &[(win.clone(), frame(50))]),
            ("com.example.Other", &[(other.clone(), frame(400))]),
        ])),
        ..FakeBackend::default()
    };
    backend.frames.insert(win.clone(), frame(50));
    backend.frames.insert(other.clone(), frame(400));
    let mut runtime = runtime(backend);
    runtime.cycle(outputs(), &cancelled).unwrap();

    let updated = Config::parse(
        "version = 1
[[rules]]
bundle_id = 'com.example.Test'
action = 'route'
workspace = 2
[[rules]]
bundle_id = 'com.example.Other'
action = 'route'
workspace = 3",
    )
    .unwrap();
    runtime.replace_config(updated);

    // Cycle 1: `Test` observation is unavailable while `Other` moves to workspace 3.
    let saved_obs = runtime.backend.observation.clone();
    runtime.backend.observation = Some(crate::NativeObservation {
        cycle: 0,
        mouse_down: false,
        applications: vec![
            crate::AppObservation::Unavailable {
                app: win.app.clone(),
                error: "transient AX timeout".into(),
            },
            crate::AppObservation::Complete {
                app: crate::ObservedApplication {
                    id: other.app.clone(),
                    bundle_id: "com.example.Other".into(),
                },
                focused: None,
                windows: vec![crate::ObservedWindow {
                    id: other.clone(),
                    frame: frame(400),
                    resizable: true,
                    title: None,
                }],
            },
        ],
    });
    runtime.cycle(outputs(), &cancelled).unwrap();
    assert_eq!(
        runtime.workspaces.workspace_of(&win),
        Some(WorkspaceId::ONE)
    );
    assert_eq!(
        runtime.workspaces.workspace_of(&other),
        Some(WorkspaceId::THREE)
    );

    // User manually moves `Other` to workspace 4 before retry runs; retry must
    // only finish `Test` and leave `Other` on workspace 4.
    let _ = runtime.workspaces.move_window(&other, WorkspaceId::FOUR);

    // Cycle 2: `Test` observation returns, but parking temporarily fails.
    runtime.backend.observation = saved_obs;
    runtime.backend.position_outcome = Some(macos::ApplyOutcome::Unavailable);
    runtime.cycle(outputs(), &cancelled).unwrap();
    assert_eq!(
        runtime.workspaces.workspace_of(&win),
        Some(WorkspaceId::ONE)
    );
    assert_eq!(
        runtime.workspaces.workspace_of(&other),
        Some(WorkspaceId::FOUR)
    );

    // Cycle 3: parking succeeds on retry and moves `Test` to workspace 2 without touching `Other`.
    runtime.backend.position_outcome = None;
    runtime.cycle(outputs(), &cancelled).unwrap();
    assert_eq!(
        runtime.workspaces.workspace_of(&win),
        Some(WorkspaceId::TWO)
    );
    assert!(runtime.parked.contains_key(&win));
    assert_eq!(
        runtime.workspaces.workspace_of(&other),
        Some(WorkspaceId::FOUR)
    );
}
