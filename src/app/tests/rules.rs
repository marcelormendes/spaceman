use super::*;

#[test]
fn new_ghostty_window_opens_on_its_current_display_active_workspace() {
    let existing = window(10);
    let created = window(11);
    let main_frame = frame(50);
    let secondary_frame = frame(1050);
    let mut backend = FakeBackend {
        focused: Some(created.clone()),
        frontmost: Some(created.app.clone()),
        observation: Some(observation_for_windows(
            "com.mitchellh.ghostty",
            &created,
            &[
                (existing.clone(), main_frame),
                (created.clone(), secondary_frame),
            ],
        )),
        ..FakeBackend::default()
    };
    backend.frames.insert(existing.clone(), main_frame);
    backend.frames.insert(created.clone(), secondary_frame);
    let mut runtime = runtime(backend);
    bind(&mut runtime, WorkspaceId::ONE, &existing);

    runtime.cycle(outputs(), &AtomicBool::new(false)).unwrap();

    assert_eq!(
        runtime.workspaces.workspace_of(&existing),
        Some(WorkspaceId::ONE)
    );
    assert_eq!(
        runtime.workspaces.workspace_of(&created),
        Some(WorkspaceId::SIX)
    );
    assert!(runtime.backend.frames[&created].x >= 1000);
}

#[test]
fn bundle_route_applies_to_initial_window_while_secondary_windows_stay_on_active_workspace() {
    let existing = window(10);
    let created = window(11);
    let mut backend = FakeBackend {
        observation: Some(observation_for_windows(
            "com.example.Test",
            &created,
            &[(existing.clone(), frame(50)), (created.clone(), frame(100))],
        )),
        ..FakeBackend::default()
    };
    backend.frames.insert(existing.clone(), frame(50));
    backend.frames.insert(created.clone(), frame(100));
    let config = Config::parse(
        "version = 1\n[[rules]]\nbundle_id = 'com.example.Test'\naction = 'route'\nworkspace = 6",
    )
    .unwrap();
    let mut runtime = runtime_with_config(backend, config);
    bind(&mut runtime, WorkspaceId::ONE, &existing);

    runtime.cycle(outputs(), &AtomicBool::new(false)).unwrap();

    assert_eq!(
        runtime.workspaces.workspace_of(&existing),
        Some(WorkspaceId::ONE)
    );
    assert_eq!(
        runtime.workspaces.workspace_of(&created),
        Some(WorkspaceId::ONE)
    );
    assert!(runtime.backend.frames[&created].x < 1000);
}

#[test]
fn title_route_rule_routes_matching_secondary_window() {
    let existing = window(10);
    let created = window(11);
    let mut observed = observation_for_windows(
        "com.example.Test",
        &created,
        &[(existing.clone(), frame(50)), (created.clone(), frame(100))],
    );
    if let crate::AppObservation::Complete { windows, .. } = &mut observed.applications[0] {
        windows[1].title = Some("Huddle".into());
    }
    let mut backend = FakeBackend {
        observation: Some(observed),
        ..FakeBackend::default()
    };
    backend.frames.insert(existing.clone(), frame(50));
    backend.frames.insert(created.clone(), frame(100));
    let config = Config::parse(
        "version = 1\n[[rules]]\nbundle_id = 'com.example.Test'\ntitle = 'Huddle'\naction = 'route'\nworkspace = 6",
    )
    .unwrap();
    let mut runtime = runtime_with_config(backend, config);
    bind(&mut runtime, WorkspaceId::ONE, &existing);

    runtime.cycle(outputs(), &AtomicBool::new(false)).unwrap();

    assert_eq!(
        runtime.workspaces.workspace_of(&existing),
        Some(WorkspaceId::ONE)
    );
    assert_eq!(
        runtime.workspaces.workspace_of(&created),
        Some(WorkspaceId::SIX)
    );
    assert!(runtime.backend.frames[&created].x >= 1000);
}

#[test]
fn floating_rule_converts_existing_and_new_windows_to_floating_ownership() {
    let existing = window(10);
    let created = window(11);
    let mut backend = FakeBackend {
        observation: Some(observation_for_windows(
            "com.example.Test",
            &created,
            &[
                (existing.clone(), frame(50)),
                (created.clone(), frame(1050)),
            ],
        )),
        ..FakeBackend::default()
    };
    backend.frames.insert(existing.clone(), frame(50));
    backend.frames.insert(created.clone(), frame(1050));
    let config =
        Config::parse("version = 1\n[[rules]]\nbundle_id = 'com.example.Test'\naction = 'float'")
            .unwrap();
    let mut runtime = runtime_with_config(backend, config);
    bind(&mut runtime, WorkspaceId::ONE, &existing);

    runtime.cycle(outputs(), &AtomicBool::new(false)).unwrap();

    assert_eq!(runtime.workspaces.workspace_of(&existing), None);
    assert_eq!(
        runtime.workspaces.floating_workspace_of(&existing),
        Some(WorkspaceId::ONE)
    );
    assert_eq!(runtime.workspaces.workspace_of(&created), None);
    assert_eq!(
        runtime.workspaces.floating_workspace_of(&created),
        Some(WorkspaceId::SIX)
    );
}

#[test]
fn ignore_rule_releases_the_window_and_never_adopts_it() {
    let existing = window(10);
    let ignored = window(11);
    let mut backend = FakeBackend {
        observation: Some(observation_for_windows(
            "com.example.Test",
            &existing,
            &[(existing.clone(), frame(50)), (ignored.clone(), frame(400))],
        )),
        ..FakeBackend::default()
    };
    backend.frames.insert(existing.clone(), frame(50));
    backend.frames.insert(ignored.clone(), frame(400));
    let config =
        Config::parse("version = 1\n[[rules]]\nbundle_id = 'com.example.Test'\naction = 'ignore'")
            .unwrap();
    let mut runtime = runtime_with_config(backend, config);
    bind(&mut runtime, WorkspaceId::ONE, &existing);

    runtime.cycle(outputs(), &AtomicBool::new(false)).unwrap();

    // Both windows match the ignore rule: the owned one is released and
    // the new one is never adopted; neither is ever parked.
    assert_eq!(runtime.workspaces.workspace_of_any(&existing), None);
    assert_eq!(runtime.workspaces.workspace_of_any(&ignored), None);
    assert!(runtime.parked.is_empty());
}

#[test]
fn app_spanning_workspaces_keeps_current_display_routing() {
    let main = window(10);
    let secondary = window(11);
    let created = window(12);
    let mut backend = FakeBackend {
        observation: Some(observation_for_windows(
            "com.example.Test",
            &created,
            &[
                (main.clone(), frame(50)),
                (secondary.clone(), frame(1050)),
                (created.clone(), frame(1100)),
            ],
        )),
        ..FakeBackend::default()
    };
    backend.frames.insert(main.clone(), frame(50));
    backend.frames.insert(secondary.clone(), frame(1050));
    backend.frames.insert(created.clone(), frame(1100));
    let mut runtime = runtime(backend);
    bind(&mut runtime, WorkspaceId::ONE, &main);
    bind(&mut runtime, WorkspaceId::SIX, &secondary);

    runtime.cycle(outputs(), &AtomicBool::new(false)).unwrap();

    assert_eq!(
        runtime.workspaces.workspace_of(&created),
        Some(WorkspaceId::SIX)
    );
}

#[test]
fn simultaneous_new_windows_stay_on_their_display_active_workspace() {
    let existing = window(10);
    let first = window(11);
    let second = window(12);
    let mut backend = FakeBackend {
        observation: Some(observation_for_windows(
            "com.example.Test",
            &second,
            &[
                (existing.clone(), frame(50)),
                (first.clone(), frame(1050)),
                (second.clone(), frame(1100)),
            ],
        )),
        ..FakeBackend::default()
    };
    backend.frames.insert(existing.clone(), frame(50));
    backend.frames.insert(first.clone(), frame(1050));
    backend.frames.insert(second.clone(), frame(1100));
    let mut runtime = runtime(backend);
    bind(&mut runtime, WorkspaceId::ONE, &existing);

    runtime.cycle(outputs(), &AtomicBool::new(false)).unwrap();

    assert_eq!(
        runtime.workspaces.workspace_of(&first),
        Some(WorkspaceId::SIX)
    );
    assert_eq!(
        runtime.workspaces.workspace_of(&second),
        Some(WorkspaceId::SIX)
    );
    assert!(runtime.backend.frames[&first].x >= 1000);
    assert!(runtime.backend.frames[&second].x >= 1000);
}

#[test]
fn new_window_opens_on_active_workspace_even_when_app_has_window_on_inactive_workspace() {
    let existing = window(10);
    let created = window(11);
    let mut backend = FakeBackend {
        observation: Some(observation_for_windows(
            "com.example.Test",
            &created,
            &[
                (existing.clone(), frame(50)),
                (created.clone(), frame(1050)),
            ],
        )),
        ..FakeBackend::default()
    };
    backend.frames.insert(existing.clone(), frame(50));
    backend.frames.insert(created.clone(), frame(1050));
    let mut runtime = runtime(backend);
    let inactive = WorkspaceId::new(2).unwrap();
    bind(&mut runtime, inactive, &existing);

    runtime.cycle(outputs(), &AtomicBool::new(false)).unwrap();

    assert_eq!(
        runtime.workspaces.workspace_of(&created),
        Some(WorkspaceId::SIX)
    );
    assert!(!runtime.parked.contains_key(&created));
    assert!(
        outputs()
            .secondary()
            .unwrap()
            .frame
            .contains_center(runtime.backend.frames[&created])
    );
}

#[test]
fn floating_rule_owns_window_without_adding_it_to_layout() {
    let window = window(10);
    let normal = frame(50);
    let mut backend = FakeBackend {
        observation: Some(observation(&window, normal)),
        ..FakeBackend::default()
    };
    backend.frames.insert(window.clone(), normal);
    let config =
        Config::parse("version = 1\n[[rules]]\nbundle_id = 'com.example.Test'\naction = 'float'")
            .unwrap();
    let mut runtime = runtime_with_config(backend, config);

    runtime.cycle(outputs(), &AtomicBool::new(false)).unwrap();

    assert!(
        runtime
            .workspaces
            .engine(WorkspaceId::ONE)
            .model()
            .workspace
            .slots
            .is_empty()
    );
    assert_eq!(runtime.workspaces.workspace_of(&window), None);
    assert_eq!(
        runtime.workspaces.floating_workspace_of(&window),
        Some(WorkspaceId::ONE)
    );
    assert!(
        !runtime
            .backend
            .calls
            .iter()
            .any(|call| matches!(call, BackendCall::Apply(_, _)))
    );
}

#[test]
fn tiled_window_keeps_its_tile_through_modal_sheet_non_resizable_state() {
    let first = window(10);
    let sibling = window(11);
    let cancelled = AtomicBool::new(false);
    let mut backend = FakeBackend {
        focused: Some(first.clone()),
        frontmost: Some(first.app.clone()),
        observation: Some(observation_for_windows(
            "com.example.Test",
            &first,
            &[(first.clone(), frame(50)), (sibling.clone(), frame(550))],
        )),
        ..FakeBackend::default()
    };
    backend.frames.insert(first.clone(), frame(50));
    backend.frames.insert(sibling.clone(), frame(550));
    let mut runtime = runtime(backend);

    runtime.cycle(outputs(), &cancelled).unwrap();
    runtime
        .resize_focused(ResizeDirection::Grow, outputs(), &cancelled)
        .unwrap();

    let snapshot_slots = |runtime: &WorkspaceRuntime<FakeBackend>| {
        runtime
            .workspaces
            .engine(WorkspaceId::ONE)
            .model()
            .workspace
            .slots
            .iter()
            .map(|slot| (slot.id, slot.bound_window().cloned(), slot.weight))
            .collect::<Vec<_>>()
    };
    let slots_before = snapshot_slots(&runtime);
    let first_frame = runtime.backend.frames[&first];
    let sibling_frame = runtime.backend.frames[&sibling];
    assert!(first_frame.width > sibling_frame.width);
    assert!(sibling_frame.width < outputs().main().frame.width);

    // A window-modal close/save sheet temporarily flips the tiled window non-resizable.
    let Some(crate::NativeObservation { applications, .. }) = &mut runtime.backend.observation
    else {
        unreachable!();
    };
    let crate::AppObservation::Complete { windows, .. } = &mut applications[0] else {
        unreachable!();
    };
    windows[0].resizable = false;

    runtime.cycle(outputs(), &cancelled).unwrap();

    assert_eq!(
        runtime.workspaces.workspace_of(&first),
        Some(WorkspaceId::ONE)
    );
    assert_eq!(runtime.workspaces.floating_workspace_of(&first), None);
    assert_eq!(snapshot_slots(&runtime), slots_before);
    assert_eq!(runtime.backend.frames[&first], first_frame);
    assert_eq!(runtime.backend.frames[&sibling], sibling_frame);

    // When the sheet closes and resizable returns to true, tiling resumes normally.
    let Some(crate::NativeObservation { applications, .. }) = &mut runtime.backend.observation
    else {
        unreachable!();
    };
    let crate::AppObservation::Complete { windows, .. } = &mut applications[0] else {
        unreachable!();
    };
    windows[0].resizable = true;

    runtime.cycle(outputs(), &cancelled).unwrap();
    assert_eq!(snapshot_slots(&runtime), slots_before);
    assert_eq!(runtime.backend.frames[&first], first_frame);
    assert_eq!(runtime.backend.frames[&sibling], sibling_frame);

    runtime
        .resize_focused(ResizeDirection::Grow, outputs(), &cancelled)
        .unwrap();
    assert!(runtime.backend.frames[&first].width > first_frame.width);
}

#[test]
fn newly_opened_non_resizable_dialog_floats_without_tiling() {
    let tiled = window(10);
    let dialog = window(11);
    let dialog_frame = frame(300);
    let mut observed = observation_for_windows(
        "com.example.Test",
        &dialog,
        &[(tiled.clone(), frame(50)), (dialog.clone(), dialog_frame)],
    );
    let crate::AppObservation::Complete { windows, .. } = &mut observed.applications[0] else {
        unreachable!();
    };
    windows[1].resizable = false;
    let mut backend = FakeBackend {
        observation: Some(observed),
        ..FakeBackend::default()
    };
    backend.frames.insert(tiled.clone(), frame(50));
    backend.frames.insert(dialog.clone(), dialog_frame);
    let mut runtime = runtime(backend);
    bind(&mut runtime, WorkspaceId::ONE, &tiled);

    runtime.cycle(outputs(), &AtomicBool::new(false)).unwrap();

    assert_eq!(
        runtime.workspaces.workspace_of(&tiled),
        Some(WorkspaceId::ONE)
    );
    assert_eq!(runtime.workspaces.workspace_of(&dialog), None);
    assert_eq!(
        runtime.workspaces.floating_workspace_of(&dialog),
        Some(WorkspaceId::ONE)
    );
    assert_eq!(runtime.backend.frames[&tiled], outputs().main().frame);
    assert_eq!(runtime.backend.frames[&dialog], dialog_frame);
    assert!(
        !runtime
            .backend
            .calls
            .iter()
            .any(|call| matches!(call, BackendCall::Apply(window, _) if window == &dialog))
    );
}

#[test]
fn manual_float_and_float_rule_still_release_tiled_windows_immediately() {
    let first = window(10);
    let sibling = window(11);
    let cancelled = AtomicBool::new(false);
    let mut backend = FakeBackend {
        focused: Some(first.clone()),
        frontmost: Some(first.app.clone()),
        observation: Some(observation_for_windows(
            "com.example.Test",
            &first,
            &[(first.clone(), frame(50)), (sibling.clone(), frame(550))],
        )),
        ..FakeBackend::default()
    };
    backend.frames.insert(first.clone(), frame(50));
    backend.frames.insert(sibling.clone(), frame(550));
    let mut runtime = runtime(backend);

    runtime.cycle(outputs(), &cancelled).unwrap();
    runtime.toggle_float(outputs(), &cancelled).unwrap();

    assert_eq!(runtime.workspaces.workspace_of(&first), None);
    assert_eq!(
        runtime.workspaces.floating_workspace_of(&first),
        Some(WorkspaceId::ONE)
    );
    assert!(runtime.manual_floating.contains(&first));
    assert_eq!(runtime.backend.frames[&sibling], outputs().main().frame);

    // Subsequent cycles keep the manually floated window floating.
    runtime.cycle(outputs(), &cancelled).unwrap();
    assert_eq!(runtime.workspaces.workspace_of(&first), None);
    assert_eq!(
        runtime.workspaces.floating_workspace_of(&first),
        Some(WorkspaceId::ONE)
    );

    // A float rule also releases the remaining tiled window immediately.
    let float_config = Config::parse(
        "version = 1
[[rules]]
bundle_id = 'com.example.Test'
action = 'float'",
    )
    .unwrap();
    runtime.replace_config(float_config);
    runtime.cycle(outputs(), &cancelled).unwrap();

    assert_eq!(runtime.workspaces.workspace_of(&sibling), None);
    assert_eq!(
        runtime.workspaces.floating_workspace_of(&sibling),
        Some(WorkspaceId::ONE)
    );
}

#[test]
fn route_rule_moves_only_inserted_window_to_inactive_workspace() {
    let window = window(10);
    let normal = frame(50);
    let mut backend = FakeBackend {
        observation: Some(observation(&window, normal)),
        ..FakeBackend::default()
    };
    backend.frames.insert(window.clone(), normal);
    let config = Config::parse(
        "version = 1\n[[rules]]\nbundle_id = 'com.example.Test'\naction = 'route'\nworkspace = 2",
    )
    .unwrap();
    let mut runtime = runtime_with_config(backend, config);

    runtime.cycle(outputs(), &AtomicBool::new(false)).unwrap();

    assert_eq!(
        runtime.workspaces.workspace_of(&window),
        Some(WorkspaceId::new(2).unwrap())
    );
    assert!(runtime.parked.contains_key(&window));
    assert!(runtime.backend.workspace_state.is_some());
    assert!(
        runtime
            .workspaces
            .engine(WorkspaceId::ONE)
            .bound_windows()
            .is_empty()
    );
}

#[test]
fn cross_display_route_survives_later_reconciliation_in_same_cycle() {
    let window = window(10);
    let normal = frame(50);
    let mut backend = FakeBackend {
        observation: Some(observation(&window, normal)),
        ..FakeBackend::default()
    };
    backend.frames.insert(window.clone(), normal);
    let config = Config::parse(
        "version = 1\n[[rules]]\nbundle_id = 'com.example.Test'\naction = 'route'\nworkspace = 6",
    )
    .unwrap();
    let mut runtime = runtime_with_config(backend, config);

    runtime.cycle(outputs(), &AtomicBool::new(false)).unwrap();

    assert_eq!(
        runtime.workspaces.workspace_of(&window),
        Some(WorkspaceId::SIX)
    );
    assert!(runtime.backend.frames[&window].x >= 1000);
}

#[test]
fn unmanaged_route_is_rejected_before_insertion() {
    let window = window(10);
    let normal = frame(50);
    let mut backend = FakeBackend {
        observation: Some(observation(&window, normal)),
        ..FakeBackend::default()
    };
    backend.frames.insert(window.clone(), normal);
    let config = Config::parse(
        "version = 1\n[[rules]]\nbundle_id = 'com.example.Test'\naction = 'route'\nworkspace = 6",
    )
    .unwrap();
    let mut runtime = WorkspaceRuntime::with_backend_and_config(
        ManagementScope::Main,
        backend,
        HashMap::new(),
        config,
    );

    assert!(runtime.cycle(outputs(), &AtomicBool::new(false)).is_err());
    assert_eq!(runtime.workspaces.workspace_of(&window), None);
}

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
        .map(|e| e.workspace)
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
        .map(|e| e.workspace)
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
            .any(|e| e.workspace == named)
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
fn rule_reload_retries_after_transient_park_or_observation_failure() {
    let win = window(10);
    let cancelled = AtomicBool::new(false);
    let mut backend = FakeBackend {
        observation: Some(observation(&win, frame(50))),
        ..FakeBackend::default()
    };
    backend.frames.insert(win.clone(), frame(50));
    let mut runtime = runtime(backend);
    runtime.cycle(outputs(), &cancelled).unwrap();
    assert_eq!(
        runtime.workspaces.workspace_of(&win),
        Some(WorkspaceId::ONE)
    );

    let to_two = Config::parse(
        "version = 1\n[[rules]]\nbundle_id = 'com.example.Test'\naction = 'route'\nworkspace = 2",
    )
    .unwrap();
    runtime.replace_config(to_two);

    // Cycle 1: app observation is temporarily unavailable, so reload defers cleanly.
    let saved_obs = runtime.backend.observation.clone();
    runtime.backend.observation = Some(crate::NativeObservation {
        cycle: 0,
        mouse_down: false,
        applications: vec![crate::AppObservation::Unavailable {
            app: win.app.clone(),
            error: "transient AX timeout".into(),
        }],
    });
    runtime.cycle(outputs(), &cancelled).unwrap();
    assert_eq!(
        runtime.workspaces.workspace_of(&win),
        Some(WorkspaceId::ONE)
    );

    // Cycle 2: observation returns, but parking temporarily fails.
    runtime.backend.observation = saved_obs;
    runtime.backend.position_outcome = Some(macos::ApplyOutcome::Unavailable);
    runtime.cycle(outputs(), &cancelled).unwrap();
    assert_eq!(
        runtime.workspaces.workspace_of(&win),
        Some(WorkspaceId::ONE)
    );

    // Cycle 3: parking succeeds on retry and completes the move to workspace 2.
    runtime.backend.position_outcome = None;
    runtime.cycle(outputs(), &cancelled).unwrap();
    assert_eq!(
        runtime.workspaces.workspace_of(&win),
        Some(WorkspaceId::TWO)
    );
    assert!(runtime.parked.contains_key(&win));
}
