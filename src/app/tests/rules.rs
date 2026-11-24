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

#[test]
fn focused_window_routed_to_hidden_workspace_suppresses_focus_follow() {
    let cancelled = AtomicBool::new(false);

    // Part A: focused non-resizable window routed to hidden workspace 2 stays parked on the next
    // cycle instead of focus-following to workspace 2.
    let settings = window(10);
    let mut observed = observation_for("com.apple.systempreferences", &settings, frame(50));
    if let crate::AppObservation::Complete { windows, .. } = &mut observed.applications[0] {
        windows[0].resizable = false;
    }
    let mut backend = FakeBackend {
        focused: Some(settings.clone()),
        frontmost: Some(settings.app.clone()),
        observation: Some(observed),
        ..FakeBackend::default()
    };
    backend.frames.insert(settings.clone(), frame(50));
    let config = Config::parse(
        "version = 1\n[[rules]]\nbundle_id = 'com.apple.systempreferences'\naction = 'route'\nworkspace = 2",
    )
    .unwrap();
    let mut runtime = runtime_with_config(backend, config);

    runtime.cycle(outputs(), &cancelled).unwrap();
    assert_eq!(
        runtime.workspaces.floating_workspace_of(&settings),
        Some(WorkspaceId::TWO)
    );
    assert_eq!(runtime.suppressed_focus, Some(settings.clone()));
    assert_eq!(runtime.suppressed_app, Some(settings.app.clone()));

    runtime.cycle(outputs(), &cancelled).unwrap();
    assert_eq!(
        runtime.workspaces.active_for(&DisplaySelector::Main),
        Some(WorkspaceId::ONE)
    );
    assert!(runtime.parked.contains_key(&settings));

    // Part B: focused resizable window routed to hidden workspace 2 also suppresses focus follow.
    let chrome = window(20);
    let mut backend = FakeBackend {
        focused: Some(chrome.clone()),
        frontmost: Some(chrome.app.clone()),
        observation: Some(observation_for("com.google.Chrome", &chrome, frame(50))),
        ..FakeBackend::default()
    };
    backend.frames.insert(chrome.clone(), frame(50));
    let config = Config::parse(
        "version = 1\n[[rules]]\nbundle_id = 'com.google.Chrome'\naction = 'route'\nworkspace = 2",
    )
    .unwrap();
    let mut runtime = runtime_with_config(backend, config);

    runtime.cycle(outputs(), &cancelled).unwrap();
    assert_eq!(
        runtime.workspaces.workspace_of(&chrome),
        Some(WorkspaceId::TWO)
    );
    assert_eq!(runtime.suppressed_focus, Some(chrome.clone()));
    assert_eq!(runtime.suppressed_app, Some(chrome.app.clone()));

    runtime.cycle(outputs(), &cancelled).unwrap();
    assert_eq!(
        runtime.workspaces.active_for(&DisplaySelector::Main),
        Some(WorkspaceId::ONE)
    );
    assert!(runtime.parked.contains_key(&chrome));
}

#[test]
fn failed_cross_display_floating_carry_retries_on_adoption_and_rule_reload() {
    let cancelled = AtomicBool::new(false);

    // Part A: adoption defers ownership when cross-display carry fails, then succeeds on retry.
    let settings = window(10);
    let mut observed = observation_for("com.apple.systempreferences", &settings, frame(50));
    if let crate::AppObservation::Complete { windows, .. } = &mut observed.applications[0] {
        windows[0].resizable = false;
    }
    let mut backend = FakeBackend {
        observation: Some(observed),
        position_outcome: Some(macos::ApplyOutcome::Unavailable),
        ..FakeBackend::default()
    };
    backend.frames.insert(settings.clone(), frame(50));
    let route_six = Config::parse(
        "version = 1\n[[rules]]\nbundle_id = 'com.apple.systempreferences'\naction = 'route'\nworkspace = 6",
    )
    .unwrap();
    let mut runtime = runtime_with_config(backend, route_six.clone());

    let status = runtime.cycle(outputs(), &cancelled).unwrap();
    assert_eq!(status.failed, 1);
    assert_eq!(runtime.workspaces.floating_workspace_of(&settings), None);

    runtime.backend.position_outcome = None;
    let status = runtime.cycle(outputs(), &cancelled).unwrap();
    assert_eq!(status.failed, 0);
    assert_eq!(
        runtime.workspaces.floating_workspace_of(&settings),
        Some(WorkspaceId::SIX)
    );
    assert!(
        outputs()
            .secondary()
            .unwrap()
            .frame
            .contains_center(runtime.backend.frames[&settings])
    );

    // Part B: live rule reload defers ownership and queues retry when cross-display carry fails.
    let mut observed = observation_for("com.apple.systempreferences", &settings, frame(50));
    if let crate::AppObservation::Complete { windows, .. } = &mut observed.applications[0] {
        windows[0].resizable = false;
    }
    let mut backend = FakeBackend {
        observation: Some(observed),
        ..FakeBackend::default()
    };
    backend.frames.insert(settings.clone(), frame(50));
    let mut runtime = runtime(backend);
    runtime.cycle(outputs(), &cancelled).unwrap();
    assert_eq!(
        runtime.workspaces.floating_workspace_of(&settings),
        Some(WorkspaceId::ONE)
    );

    runtime.replace_config(route_six);
    runtime.backend.position_outcome = Some(macos::ApplyOutcome::Unavailable);
    let status = runtime.cycle(outputs(), &cancelled).unwrap();
    assert_eq!(status.failed, 1);
    assert_eq!(
        runtime.workspaces.floating_workspace_of(&settings),
        Some(WorkspaceId::ONE)
    );

    runtime.backend.position_outcome = None;
    let status = runtime.cycle(outputs(), &cancelled).unwrap();
    assert_eq!(status.failed, 0);
    assert_eq!(
        runtime.workspaces.floating_workspace_of(&settings),
        Some(WorkspaceId::SIX)
    );
    assert!(
        outputs()
            .secondary()
            .unwrap()
            .frame
            .contains_center(runtime.backend.frames[&settings])
    );
}

#[test]
fn reassigned_workspace_retries_failed_recovery_and_floating_carry_idempotently() {
    let cancelled = AtomicBool::new(false);
    let seven = WorkspaceId::new(7).unwrap();
    let nine = WorkspaceId::new(9).unwrap();

    // Part A: failed recovery persistence retains the reassignment and retries next cycle.
    let notes = window(10);
    let secondary_displays: Vec<_> = outputs().frames().collect();
    let secondary_frame = outputs().secondary().unwrap().frame;
    let notes_orig = frame(1100);
    let mut backend = FakeBackend {
        observation: Some(crate::NativeObservation {
            cycle: 0,
            mouse_down: false,
            applications: Vec::new(),
        }),
        fail_persist: true,
        ..FakeBackend::default()
    };
    backend.frames.insert(
        notes.clone(),
        crate::geometry::parked_frame(secondary_frame, &secondary_displays, notes_orig),
    );
    let mut runtime = runtime_with_parked(backend, [(notes.clone(), notes_orig)]);
    runtime.workspaces.switch(seven);
    bind(&mut runtime, WorkspaceId::SIX, &notes);
    let to_main =
        Config::parse("version = 1\n[[workspaces]]\nworkspace = 6\nmonitor = 'main'").unwrap();
    runtime.replace_config(to_main);

    let status = runtime.cycle(outputs(), &cancelled).unwrap();
    assert_eq!(status.failed, 1);
    assert_eq!(runtime.parked[&notes], notes_orig);

    runtime.backend.fail_persist = false;
    let status = runtime.cycle(outputs(), &cancelled).unwrap();
    assert_eq!(status.failed, 0);
    assert!(
        outputs()
            .main()
            .frame
            .contains_center(runtime.parked[&notes])
    );

    // Part B: failed visible floating carry retains the reassignment and retries idempotently.
    let first = window(20);
    let second = window(21);
    let three = three_outputs();
    let mut observed = observation_for_windows(
        "com.example.Test",
        &first,
        &[(first.clone(), frame(1050)), (second.clone(), frame(1100))],
    );
    if let crate::AppObservation::Complete { windows, .. } = &mut observed.applications[0] {
        windows[0].resizable = false;
        windows[1].resizable = false;
    }
    let mut backend = FakeBackend {
        observation: Some(observed),
        position_outcome: Some(macos::ApplyOutcome::Unavailable),
        ..FakeBackend::default()
    };
    backend.frames.insert(first.clone(), frame(1050));
    backend.frames.insert(second.clone(), frame(1100));
    let mut runtime = runtime(backend);
    runtime.workspaces.switch(nine);
    runtime.workspaces.own_floating(first.clone(), nine);
    runtime.workspaces.own_floating(second.clone(), nine);

    let to_third =
        Config::parse("version = 1\n[[workspaces]]\nworkspace = 9\nmonitor = 'third-id'").unwrap();
    runtime.replace_config(to_third);

    let status = runtime.cycle(three.clone(), &cancelled).unwrap();
    assert_eq!(status.failed, 2);
    assert!(
        three
            .secondary()
            .unwrap()
            .frame
            .contains_center(runtime.backend.frames[&first])
    );

    runtime.backend.position_outcome = None;
    let status = runtime.cycle(three.clone(), &cancelled).unwrap();
    assert_eq!(status.failed, 0);
    let third_frame = three.displays()[2].frame;
    assert!(third_frame.contains_center(runtime.backend.frames[&first]));
    assert!(third_frame.contains_center(runtime.backend.frames[&second]));

    let calls_before = runtime.backend.calls.len();
    let status = runtime.cycle(three, &cancelled).unwrap();
    assert_eq!(status.failed, 0);
    assert_eq!(runtime.backend.calls.len(), calls_before);
}

#[test]
fn parked_floating_window_reloaded_across_displays_to_visible_lands_at_translated_frame() {
    let settings = window(10);
    let original = frame(50);
    let cancelled = AtomicBool::new(false);
    let mut backend = FakeBackend {
        observation: Some(crate::NativeObservation {
            cycle: 0,
            mouse_down: false,
            applications: Vec::new(),
        }),
        ..FakeBackend::default()
    };
    backend
        .frames
        .insert(settings.clone(), main_parked_frame(original));
    let route_two = Config::parse(
        "version = 1\n[[rules]]\nbundle_id = 'com.apple.systempreferences'\naction = 'route'\nworkspace = 2",
    )
    .unwrap();
    let mut runtime = WorkspaceRuntime::with_backend_config_state(
        ManagementScope::Both,
        backend,
        HashMap::from([(settings.clone(), original)]),
        route_two,
        None,
    );
    runtime
        .workspaces
        .own_floating(settings.clone(), WorkspaceId::TWO);
    runtime
        .memory
        .bundles
        .insert(settings.app.clone(), "com.apple.systempreferences".into());

    let route_six = Config::parse(
        "version = 1\n[[rules]]\nbundle_id = 'com.apple.systempreferences'\naction = 'route'\nworkspace = 6",
    )
    .unwrap();
    runtime.replace_config(route_six);
    let status = runtime.cycle(outputs(), &cancelled).unwrap();

    assert_eq!(status.failed, 0);
    assert_eq!(
        runtime.workspaces.floating_workspace_of(&settings),
        Some(WorkspaceId::SIX)
    );
    assert!(!runtime.parked.contains_key(&settings));
    assert_eq!(runtime.backend.frames[&settings], frame(1050));
}

#[test]
fn failed_sibling_initial_route_retries_even_after_first_sibling_succeeds() {
    let first = window(10);
    let second = window(11);
    let first_frame = frame(50);
    let second_frame = crate::Rect {
        x: 300,
        y: 20,
        width: 500,
        height: 300,
    };
    let cancelled = AtomicBool::new(false);
    let mut backend = FakeBackend {
        observation: Some(observation_for_windows(
            "com.google.Chrome",
            &first,
            &[(first.clone(), first_frame), (second.clone(), second_frame)],
        )),
        refuses_position: Some(|target| target.width == 500),
        ..FakeBackend::default()
    };
    backend.frames.insert(first.clone(), first_frame);
    backend.frames.insert(second.clone(), second_frame);
    let config = Config::parse(
        "version = 1\n[[rules]]\nbundle_id = 'com.google.Chrome'\naction = 'route'\nworkspace = 2",
    )
    .unwrap();
    let mut runtime = runtime_with_config(backend, config);

    // Cycle 1: first sibling routes to workspace 2 (so app now owns a window);
    // second sibling's park fails transiently and stays queued for retry.
    let status = runtime.cycle(outputs(), &cancelled).unwrap();
    assert_eq!(status.failed, 1);
    assert_eq!(
        runtime.workspaces.workspace_of(&first),
        Some(WorkspaceId::TWO)
    );
    assert_eq!(runtime.workspaces.workspace_of(&second), None);

    // Cycle 2: retry still routes the second sibling to workspace 2 instead of stranding it on 1.
    runtime.backend.refuses_position = None;
    let status = runtime.cycle(outputs(), &cancelled).unwrap();
    assert_eq!(status.failed, 0);
    assert_eq!(
        runtime.workspaces.workspace_of(&second),
        Some(WorkspaceId::TWO)
    );
    assert!(runtime.parked.contains_key(&second));
    assert!(runtime.pending_initial_routes.is_empty());
}

#[test]
fn park_inactive_cross_display_failure_keeps_persisted_recovery_frame() {
    let win = window(10);
    let cancelled = AtomicBool::new(false);
    let mut backend = FakeBackend {
        observation: Some(observation(&win, frame(1050))),
        position_outcome: Some(macos::ApplyOutcome::Unavailable),
        ..FakeBackend::default()
    };
    backend.frames.insert(win.clone(), frame(1050));
    let mut runtime = runtime(backend);
    bind(&mut runtime, WorkspaceId::SIX, &win);
    let to_main =
        Config::parse("version = 1\n[[workspaces]]\nworkspace = 6\nmonitor = 'main'").unwrap();
    runtime.replace_config(to_main);

    let status = runtime.cycle(outputs(), &cancelled).unwrap();
    assert_eq!(status.failed, 1);
    assert_eq!(runtime.parked.get(&win), Some(&frame(50)));
    assert_eq!(
        runtime.parked.get(&win),
        runtime.backend.persisted.get(&win)
    );
}
