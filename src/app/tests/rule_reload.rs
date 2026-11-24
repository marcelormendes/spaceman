use super::*;
use crate::scratchpad::ScratchpadCommand;

fn app_window(pid: i32, window_id: u32) -> crate::PhysicalWindowId {
    crate::PhysicalWindowId {
        app: AppInstanceId {
            pid,
            launched_at: pid as u64,
        },
        window_id,
    }
}

fn app_observation(
    bundle_id: &str,
    window: &crate::PhysicalWindowId,
    frame: crate::Rect,
    resizable: bool,
    title: Option<&str>,
) -> crate::AppObservation {
    crate::AppObservation::Complete {
        app: crate::ObservedApplication {
            id: window.app.clone(),
            bundle_id: bundle_id.into(),
        },
        focused: None,
        windows: vec![crate::ObservedWindow {
            id: window.clone(),
            frame,
            resizable,
            title: title.map(Arc::from),
        }],
    }
}

#[test]
fn adding_route_rule_moves_open_window_to_hidden_and_visible_workspaces() {
    let slack = app_window(10, 100);
    let slack_frame = frame(50);
    let mut backend = FakeBackend {
        focused: Some(slack.clone()),
        frontmost: Some(slack.app.clone()),
        observation: Some(crate::NativeObservation {
            cycle: 0,
            mouse_down: false,
            applications: vec![app_observation(
                "com.tinyspeck.slackmacgap",
                &slack,
                slack_frame,
                true,
                None,
            )],
        }),
        ..FakeBackend::default()
    };
    backend.frames.insert(slack.clone(), slack_frame);
    let mut rt = runtime(backend);
    let cancelled = AtomicBool::new(false);

    rt.cycle(outputs(), &cancelled).unwrap();
    assert_eq!(rt.workspaces.workspace_of(&slack), Some(WorkspaceId::ONE));

    // Add a route rule sending Slack to hidden workspace 3 on Main.
    let to_three = Config::parse(
        "version = 1\n[[rules]]\nbundle_id = 'com.tinyspeck.slackmacgap'\naction = 'route'\nworkspace = 3",
    )
    .unwrap();
    rt.replace_config(to_three);
    rt.cycle(outputs(), &cancelled).unwrap();

    assert_eq!(
        rt.workspaces.workspace_of(&slack),
        Some(WorkspaceId::new(3).unwrap())
    );
    assert!(rt.parked.contains_key(&slack));
    assert_eq!(rt.suppressed_focus.as_ref(), Some(&slack));
    // A second cycle must not focus-follow back to hidden workspace 3.
    rt.cycle(outputs(), &cancelled).unwrap();
    assert_eq!(
        rt.workspaces.active_for(&DisplaySelector::Main),
        Some(WorkspaceId::ONE)
    );

    // Edit the rule to visible workspace 6 on Secondary: Slack restores and lays out there.
    let to_six = Config::parse(
        "version = 1\n[[rules]]\nbundle_id = 'com.tinyspeck.slackmacgap'\naction = 'route'\nworkspace = 6",
    )
    .unwrap();
    rt.replace_config(to_six);
    rt.cycle(outputs(), &cancelled).unwrap();

    assert_eq!(rt.workspaces.workspace_of(&slack), Some(WorkspaceId::SIX));
    assert!(!rt.parked.contains_key(&slack));
    assert!(
        outputs()
            .secondary()
            .unwrap()
            .frame
            .contains_center(rt.backend.frames[&slack])
    );
}

#[test]
fn changing_route_rule_relocates_parked_window_across_displays_using_backend_bundle_id() {
    let slack = app_window(20, 200);
    let main_displays: Vec<_> = outputs().frames().collect();
    let main_frame = outputs().main().frame;
    let orig_frame = frame(80);
    let mut backend = FakeBackend {
        observation: Some(crate::NativeObservation {
            cycle: 0,
            mouse_down: false,
            applications: Vec::new(),
        }),
        ..FakeBackend::default()
    };
    backend.frames.insert(
        slack.clone(),
        crate::geometry::parked_frame(main_frame, &main_displays, orig_frame),
    );
    backend
        .bundle_ids
        .insert(slack.app.clone(), "com.tinyspeck.slackmacgap".into());
    let old_config = Config::parse(
        "version = 1\n[[rules]]\nbundle_id = 'com.tinyspeck.slackmacgap'\naction = 'route'\nworkspace = 2",
    )
    .unwrap();
    let mut rt = WorkspaceRuntime::with_backend_and_config(
        ManagementScope::Both,
        backend,
        HashMap::from([(slack.clone(), orig_frame)]),
        old_config,
    );
    bind(&mut rt, WorkspaceId::TWO, &slack);

    // Move the rule from hidden workspace 2 (Main) to hidden workspace 7 (Secondary).
    let seven = WorkspaceId::new(7).unwrap();
    let to_seven = Config::parse(
        "version = 1\n[[rules]]\nbundle_id = 'com.tinyspeck.slackmacgap'\naction = 'route'\nworkspace = 7",
    )
    .unwrap();
    rt.replace_config(to_seven);
    rt.cycle(outputs(), &AtomicBool::new(false)).unwrap();

    assert_eq!(rt.workspaces.workspace_of(&slack), Some(seven));
    assert!(
        outputs()
            .secondary()
            .unwrap()
            .frame
            .contains_center(rt.parked[&slack])
    );
    assert_eq!(rt.backend.persisted[&slack], rt.parked[&slack]);
}

#[test]
fn float_and_ignore_rule_edits_apply_live_and_preserve_manual_floats() {
    let tiled = app_window(30, 300);
    let manual = app_window(31, 301);
    let mut backend = FakeBackend {
        observation: Some(crate::NativeObservation {
            cycle: 0,
            mouse_down: false,
            applications: vec![
                app_observation("com.example.App", &tiled, frame(50), true, None),
                app_observation("com.example.Manual", &manual, frame(250), true, None),
            ],
        }),
        ..FakeBackend::default()
    };
    backend.frames.insert(tiled.clone(), frame(50));
    backend.frames.insert(manual.clone(), frame(250));
    let mut rt = runtime(backend);
    let cancelled = AtomicBool::new(false);

    rt.cycle(outputs(), &cancelled).unwrap();
    assert_eq!(rt.workspaces.workspace_of(&tiled), Some(WorkspaceId::ONE));
    // Manually float `manual` via Toggle Float.
    rt.backend.focused = Some(manual.clone());
    rt.toggle_float(outputs(), &cancelled).unwrap();
    assert!(rt.manual_floating.contains(&manual));
    assert_eq!(
        rt.workspaces.floating_workspace_of(&manual),
        Some(WorkspaceId::ONE)
    );

    // Add a float rule for `tiled`: it floats immediately.
    let float_cfg =
        Config::parse("version = 1\n[[rules]]\nbundle_id = 'com.example.App'\naction = 'float'")
            .unwrap();
    rt.replace_config(float_cfg);
    rt.cycle(outputs(), &cancelled).unwrap();
    assert_eq!(rt.workspaces.workspace_of(&tiled), None);
    assert_eq!(
        rt.workspaces.floating_workspace_of(&tiled),
        Some(WorkspaceId::ONE)
    );

    // Remove the float rule: `tiled` tiles again while `manual` stays floating.
    rt.replace_config(Config::default());
    rt.cycle(outputs(), &cancelled).unwrap();
    assert_eq!(rt.workspaces.workspace_of(&tiled), Some(WorkspaceId::ONE));
    assert_eq!(
        rt.workspaces.floating_workspace_of(&manual),
        Some(WorkspaceId::ONE)
    );

    // Add an ignore rule for `tiled`, then replace it with a route rule to workspace 2.
    let ignore_cfg =
        Config::parse("version = 1\n[[rules]]\nbundle_id = 'com.example.App'\naction = 'ignore'")
            .unwrap();
    rt.replace_config(ignore_cfg);
    rt.cycle(outputs(), &cancelled).unwrap();
    assert_eq!(rt.workspaces.workspace_of_any(&tiled), None);

    let route_cfg = Config::parse(
        "version = 1\n[[rules]]\nbundle_id = 'com.example.App'\naction = 'route'\nworkspace = 2",
    )
    .unwrap();
    rt.replace_config(route_cfg);
    rt.cycle(outputs(), &cancelled).unwrap();
    assert_eq!(rt.workspaces.workspace_of(&tiled), Some(WorkspaceId::TWO));
    assert!(rt.parked.contains_key(&tiled));
}

#[test]
fn unchanged_rules_and_restarted_sessions_preserve_manually_moved_windows() {
    let slack = app_window(40, 400);
    let slack_frame = frame(50);
    let rule_cfg = Config::parse(
        "version = 1\n[[rules]]\nbundle_id = 'com.tinyspeck.slackmacgap'\naction = 'route'\nworkspace = 3",
    )
    .unwrap();
    let mut backend = FakeBackend {
        observation: Some(crate::NativeObservation {
            cycle: 0,
            mouse_down: false,
            applications: vec![app_observation(
                "com.tinyspeck.slackmacgap",
                &slack,
                slack_frame,
                true,
                None,
            )],
        }),
        ..FakeBackend::default()
    };
    backend.frames.insert(slack.clone(), slack_frame);
    let mut rt = runtime_with_config(backend, rule_cfg.clone());
    let cancelled = AtomicBool::new(false);

    // Manually place Slack on workspace 1 and simulate an unrelated rule edit.
    bind(&mut rt, WorkspaceId::ONE, &slack);
    let unrelated = Config::parse(
        "version = 1\n\
         [[rules]]\nbundle_id = 'com.tinyspeck.slackmacgap'\naction = 'route'\nworkspace = 3\n\
         [[rules]]\nbundle_id = 'com.example.Other'\naction = 'float'",
    )
    .unwrap();
    rt.replace_config(unrelated);
    rt.cycle(outputs(), &cancelled).unwrap();
    assert_eq!(rt.workspaces.workspace_of(&slack), Some(WorkspaceId::ONE));

    // Restart with persisted workspace state keeping Slack on workspace 1: the existing route
    // rule to workspace 3 must NOT pull the manually moved window back on startup.
    let saved_state = rt.workspaces.state();
    let mut restart_backend = FakeBackend {
        observation: Some(crate::NativeObservation {
            cycle: 0,
            mouse_down: false,
            applications: vec![app_observation(
                "com.tinyspeck.slackmacgap",
                &slack,
                slack_frame,
                true,
                None,
            )],
        }),
        ..FakeBackend::default()
    };
    restart_backend.frames.insert(slack.clone(), slack_frame);
    let mut restarted = WorkspaceRuntime::with_backend_config_state(
        ManagementScope::Both,
        restart_backend,
        HashMap::new(),
        rule_cfg,
        Some(saved_state),
    );
    restarted.cycle(outputs(), &cancelled).unwrap();
    assert_eq!(
        restarted.workspaces.workspace_of(&slack),
        Some(WorkspaceId::ONE)
    );
    assert!(!restarted.parked.contains_key(&slack));
}

#[test]
fn rule_reload_never_touches_hidden_or_shown_scratchpad_windows() {
    let scratch = app_window(50, 500);
    let scratch_frame = frame(60);
    let lab_cfg = Config::parse("version = 1\n[labs]\nscratchpad = true\n").unwrap();
    let mut backend = FakeBackend {
        focused: Some(scratch.clone()),
        frontmost: Some(scratch.app.clone()),
        observation: Some(crate::NativeObservation {
            cycle: 0,
            mouse_down: false,
            applications: vec![app_observation(
                "com.example.Scratch",
                &scratch,
                scratch_frame,
                true,
                None,
            )],
        }),
        ..FakeBackend::default()
    };
    backend.frames.insert(scratch.clone(), scratch_frame);
    let mut rt = runtime_with_config(backend, lab_cfg);
    let cancelled = AtomicBool::new(false);

    rt.cycle(outputs(), &cancelled).unwrap();
    assert_eq!(rt.workspaces.workspace_of(&scratch), Some(WorkspaceId::ONE));

    // Move `scratch` into the Scratchpad (hidden).
    rt.execute(
        WorkerCommand::Action(WorkerAction::Scratchpad(ScratchpadCommand::Mark), outputs()),
        &cancelled,
    );
    assert!(rt.scratchpad.contains(&scratch));
    assert_eq!(rt.workspaces.workspace_of_any(&scratch), None);

    // Reload a route rule matching `com.example.Scratch`: hidden scratchpad stays untouched.
    let route_cfg = Config::parse(
        "version = 1\n[labs]\nscratchpad = true\n\
         [[rules]]\nbundle_id = 'com.example.Scratch'\naction = 'route'\nworkspace = 3",
    )
    .unwrap();
    rt.replace_config(route_cfg);
    rt.cycle(outputs(), &cancelled).unwrap();
    assert!(rt.scratchpad.contains(&scratch));
    assert_eq!(rt.scratchpad.entry(&scratch).unwrap().shown_on, None);
    assert_eq!(rt.workspaces.workspace_of_any(&scratch), None);

    // Summon `scratch` (shown on workspace 1), then reload an ignore rule and a route rule:
    // shown scratchpad stays in the scratchpad on workspace 1.
    rt.execute(
        WorkerCommand::Action(
            WorkerAction::Scratchpad(ScratchpadCommand::Toggle),
            outputs(),
        ),
        &cancelled,
    );
    assert!(rt.scratchpad.entry(&scratch).unwrap().shown_on.is_some());

    let ignore_cfg = Config::parse(
        "version = 1\n[labs]\nscratchpad = true\n\
         [[rules]]\nbundle_id = 'com.example.Scratch'\naction = 'ignore'",
    )
    .unwrap();
    rt.replace_config(ignore_cfg);
    rt.cycle(outputs(), &cancelled).unwrap();
    assert!(rt.scratchpad.contains(&scratch));
    assert!(rt.scratchpad.entry(&scratch).unwrap().shown_on.is_some());
}

#[test]
fn failed_restore_rollback_never_diverges_memory_from_disk() {
    let slack = app_window(60, 600);
    let main_displays: Vec<_> = outputs().frames().collect();
    let main_frame = outputs().main().frame;
    let orig_frame = frame(90);
    let mut backend = FakeBackend {
        observation: Some(crate::NativeObservation {
            cycle: 0,
            mouse_down: false,
            applications: Vec::new(),
        }),
        // Restore write fails, and the second persist_parked (the rollback write) also fails.
        position_outcome: Some(macos::ApplyOutcome::Unavailable),
        fail_persist_after: Some(1),
        ..FakeBackend::default()
    };
    backend.frames.insert(
        slack.clone(),
        crate::geometry::parked_frame(main_frame, &main_displays, orig_frame),
    );
    backend
        .bundle_ids
        .insert(slack.app.clone(), "com.tinyspeck.slackmacgap".into());
    let mut rt = WorkspaceRuntime::with_backend_and_config(
        ManagementScope::Both,
        backend,
        HashMap::from([(slack.clone(), orig_frame)]),
        Config::default(),
    );
    rt.backend.persisted = rt.parked.clone();
    bind(&mut rt, WorkspaceId::TWO, &slack);

    // Route Slack from hidden workspace 2 (Main) to visible workspace 6 (Secondary).
    let to_six = Config::parse(
        "version = 1\n[[rules]]\nbundle_id = 'com.tinyspeck.slackmacgap'\naction = 'route'\nworkspace = 6",
    )
    .unwrap();
    rt.replace_config(to_six);
    let status = rt.cycle(outputs(), &AtomicBool::new(false)).unwrap();

    assert!(status.failed > 0);
    assert_eq!(rt.workspaces.workspace_of(&slack), Some(WorkspaceId::TWO));
    // In-memory recovery metadata must match what was last durably persisted on disk.
    assert_eq!(rt.parked, rt.backend.persisted);
}

#[test]
fn non_resizable_and_parked_title_rule_windows_stay_conservative() {
    let settings = app_window(70, 700);
    let settings_frame = frame(70);
    let mut backend = FakeBackend {
        observation: Some(crate::NativeObservation {
            cycle: 0,
            mouse_down: false,
            applications: vec![app_observation(
                "com.apple.systempreferences",
                &settings,
                settings_frame,
                false,
                None,
            )],
        }),
        ..FakeBackend::default()
    };
    backend.frames.insert(settings.clone(), settings_frame);
    let mut rt = runtime(backend);
    let cancelled = AtomicBool::new(false);

    rt.cycle(outputs(), &cancelled).unwrap();
    assert_eq!(
        rt.workspaces.floating_workspace_of(&settings),
        Some(WorkspaceId::ONE)
    );

    let route_settings = Config::parse(
        "version = 1\n[[rules]]\nbundle_id = 'com.apple.systempreferences'\naction = 'route'\nworkspace = 2",
    )
    .unwrap();
    rt.replace_config(route_settings);
    rt.cycle(outputs(), &cancelled).unwrap();
    // Non-resizable windows stay floating on their current workspace.
    assert_eq!(
        rt.workspaces.floating_workspace_of(&settings),
        Some(WorkspaceId::ONE)
    );
    assert!(!rt.parked.contains_key(&settings));

    // A parked window whose bundle has a title rule is skipped because titles are never persisted.
    let parked_win = app_window(71, 701);
    let main_displays: Vec<_> = outputs().frames().collect();
    let main_frame = outputs().main().frame;
    let orig = frame(100);
    let mut backend = FakeBackend {
        observation: Some(crate::NativeObservation {
            cycle: 0,
            mouse_down: false,
            applications: Vec::new(),
        }),
        ..FakeBackend::default()
    };
    backend.frames.insert(
        parked_win.clone(),
        crate::geometry::parked_frame(main_frame, &main_displays, orig),
    );
    backend
        .bundle_ids
        .insert(parked_win.app.clone(), "com.example.Titled".into());
    let mut rt = WorkspaceRuntime::with_backend_and_config(
        ManagementScope::Both,
        backend,
        HashMap::from([(parked_win.clone(), orig)]),
        Config::default(),
    );
    bind(&mut rt, WorkspaceId::TWO, &parked_win);

    let title_rule = Config::parse(
        "version = 1\n[[rules]]\nbundle_id = 'com.example.Titled'\ntitle = 'Inbox'\naction = 'route'\nworkspace = 3",
    )
    .unwrap();
    rt.replace_config(title_rule);
    rt.cycle(outputs(), &cancelled).unwrap();
    assert_eq!(
        rt.workspaces.workspace_of(&parked_win),
        Some(WorkspaceId::TWO)
    );
}
