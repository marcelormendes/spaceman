use super::*;

fn external_1080p() -> macos::DisplayOutputs {
    macos::DisplayOutputs::new(vec![macos::DisplayOutput {
        id: "external-1080p".into(),
        name: "External".into(),
        frame: crate::Rect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        },
        is_main: true,
    }])
    .unwrap()
}

fn builtin_1512() -> macos::DisplayOutputs {
    macos::DisplayOutputs::new(vec![macos::DisplayOutput {
        id: "builtin-1512".into(),
        name: "Built-in".into(),
        frame: crate::Rect {
            x: 0,
            y: 0,
            width: 1512,
            height: 982,
        },
        is_main: true,
    }])
    .unwrap()
}

fn app_window(pid: i32, window_id: u32) -> crate::PhysicalWindowId {
    crate::PhysicalWindowId {
        app: AppInstanceId {
            pid,
            launched_at: 1,
        },
        window_id,
    }
}

fn bottom_right_sliver() -> crate::Rect {
    // Exact geometry from issue #150: 1px x 32px sliver visible on 1512x982,
    // center (2262, 1399) outside every connected display.
    crate::Rect {
        x: 1511,
        y: 950,
        width: 1502,
        height: 898,
    }
}

#[test]
fn visible_tiled_window_left_in_bottom_right_sliver_after_display_disconnect_keeps_slot_and_retiles()
 {
    let external = external_1080p();
    let built_in = builtin_1512();
    let cancelled = AtomicBool::new(false);

    let browser = app_window(10, 100);
    let parked_windows = [
        (WorkspaceId::ONE, app_window(20, 200)),
        (WorkspaceId::new(2).unwrap(), app_window(21, 201)),
        (WorkspaceId::new(3).unwrap(), app_window(22, 202)),
        (WorkspaceId::new(4).unwrap(), app_window(23, 203)),
    ];

    let config = Config::parse(
        "version = 1
[[workspaces]]
workspace = 'B'
monitor = 'main'",
    )
    .unwrap();
    let ws_b = WorkspaceId::parse("B").unwrap();

    let mut frames = HashMap::from([(browser.clone(), external.main().frame)]);
    let ext_displays: Vec<_> = external.frames().collect();
    let mut parked_map = HashMap::new();
    for (_, win) in &parked_windows {
        let original = crate::Rect {
            x: 0,
            y: 34,
            width: 1502,
            height: 898,
        };
        let parked_pos =
            crate::geometry::parked_frame(external.main().frame, &ext_displays, original);
        frames.insert(win.clone(), parked_pos);
        parked_map.insert(win.clone(), original);
    }

    let backend = FakeBackend {
        frames,
        focused: Some(browser.clone()),
        frontmost: Some(browser.app.clone()),
        observation: Some(observation_for(
            "com.example.Browser",
            &browser,
            external.main().frame,
        )),
        ..FakeBackend::default()
    };
    let mut runtime = WorkspaceRuntime::with_backend_and_config(
        ManagementScope::Both,
        backend,
        parked_map,
        config,
    );
    for (ws, win) in &parked_windows {
        bind(&mut runtime, *ws, win);
    }
    runtime.switch(ws_b, external.clone(), &cancelled).unwrap();
    runtime.cycle(external, &cancelled).unwrap();

    let slot_before = runtime
        .workspaces
        .engine(ws_b)
        .model()
        .workspace
        .slots
        .iter()
        .find(|slot| slot.bound_window() == Some(&browser))
        .expect("browser bound on workspace B")
        .id;

    // External display disconnected, lid opened to built-in 1512x982.
    // macOS leaves visible window A in a bottom-right sliver off-center.
    let sliver = bottom_right_sliver();
    assert!(!built_in.main().frame.contains_center(sliver));
    runtime.backend.frames.insert(browser.clone(), sliver);

    runtime.cycle(built_in.clone(), &cancelled).unwrap();

    assert_eq!(runtime.workspaces.workspace_of(&browser), Some(ws_b));
    let slot_after = runtime
        .workspaces
        .engine(ws_b)
        .model()
        .workspace
        .slots
        .iter()
        .find(|slot| slot.bound_window() == Some(&browser))
        .expect("browser kept its slot on workspace B")
        .id;
    assert_eq!(slot_after, slot_before);
    assert_eq!(runtime.backend.frames[&browser], built_in.main().frame);
    assert!(
        built_in
            .main()
            .frame
            .contains_center(runtime.backend.frames[&browser])
    );

    // Hidden parked windows remain owned by their hidden workspaces and off-screen.
    for (ws, win) in &parked_windows {
        assert_eq!(runtime.workspaces.workspace_of(win), Some(*ws));
        assert!(runtime.parked.contains_key(win));
        assert!(
            !built_in
                .main()
                .frame
                .contains_center(runtime.backend.frames[win])
        );
    }
}

#[test]
fn transient_recovery_failure_after_display_disconnect_preserves_slot_until_window_moves_back() {
    let external = external_1080p();
    let built_in = builtin_1512();
    let cancelled = AtomicBool::new(false);
    let browser = app_window(10, 100);

    let backend = FakeBackend {
        frames: HashMap::from([(browser.clone(), external.main().frame)]),
        observation: Some(observation_for(
            "com.example.Browser",
            &browser,
            external.main().frame,
        )),
        ..FakeBackend::default()
    };
    let mut runtime = runtime(backend);
    runtime.cycle(external, &cancelled).unwrap();
    let slot_before = runtime
        .workspaces
        .engine(WorkspaceId::ONE)
        .model()
        .workspace
        .slots[0]
        .id;

    // App is briefly unresponsive right after display disconnect.
    let sliver = bottom_right_sliver();
    runtime.backend.frames.insert(browser.clone(), sliver);
    runtime.backend.managed_outcome = Some(macos::ApplyOutcome::Unavailable);

    let status = runtime.cycle(built_in.clone(), &cancelled).unwrap();
    assert!(status.failed >= 1);
    assert_eq!(
        runtime.workspaces.workspace_of(&browser),
        Some(WorkspaceId::ONE),
        "on-screen stranded bound window must not lose its slot while recovery settles"
    );
    assert!(runtime.omitted_windows.contains_key(&browser));

    // App responds on the next cycle: window snaps back onto built-in display
    // with the exact same slot.
    runtime.backend.managed_outcome = None;
    let status = runtime.cycle(built_in.clone(), &cancelled).unwrap();
    assert_eq!(status.failed, 0);
    assert_eq!(
        runtime.workspaces.workspace_of(&browser),
        Some(WorkspaceId::ONE)
    );
    assert_eq!(
        runtime
            .workspaces
            .engine(WorkspaceId::ONE)
            .model()
            .workspace
            .slots[0]
            .id,
        slot_before
    );
    assert_eq!(runtime.backend.frames[&browser], built_in.main().frame);
    assert!(runtime.omitted_windows.is_empty());
}

#[test]
fn multi_window_workspace_keeps_order_when_one_tile_is_stranded_after_disconnect() {
    let external = external_1080p();
    let built_in = builtin_1512();
    let cancelled = AtomicBool::new(false);
    let left = app_window(10, 100);
    let right = app_window(10, 101);

    let backend = FakeBackend {
        frames: HashMap::from([
            (
                left.clone(),
                crate::Rect {
                    x: 0,
                    y: 0,
                    width: 960,
                    height: 1080,
                },
            ),
            (
                right.clone(),
                crate::Rect {
                    x: 960,
                    y: 0,
                    width: 960,
                    height: 1080,
                },
            ),
        ]),
        observation: Some(observation_for_windows(
            "com.example.Browser",
            &left,
            &[
                (
                    left.clone(),
                    crate::Rect {
                        x: 0,
                        y: 0,
                        width: 960,
                        height: 1080,
                    },
                ),
                (
                    right.clone(),
                    crate::Rect {
                        x: 960,
                        y: 0,
                        width: 960,
                        height: 1080,
                    },
                ),
            ],
        )),
        ..FakeBackend::default()
    };
    let mut runtime = runtime(backend);
    runtime.cycle(external, &cancelled).unwrap();
    let slots_before: Vec<_> = runtime
        .workspaces
        .engine(WorkspaceId::ONE)
        .model()
        .workspace
        .slots
        .iter()
        .map(|slot| (slot.id, slot.bound_window().cloned()))
        .collect();

    // Disconnect external display: right tile is left in the bottom-right sliver
    // while left tile remains on screen.
    runtime.backend.frames.insert(
        left.clone(),
        crate::Rect {
            x: 0,
            y: 0,
            width: 960,
            height: 982,
        },
    );
    runtime
        .backend
        .frames
        .insert(right.clone(), bottom_right_sliver());

    runtime.cycle(built_in.clone(), &cancelled).unwrap();

    let slots_after: Vec<_> = runtime
        .workspaces
        .engine(WorkspaceId::ONE)
        .model()
        .workspace
        .slots
        .iter()
        .map(|slot| (slot.id, slot.bound_window().cloned()))
        .collect();
    assert_eq!(slots_after, slots_before);
    assert_eq!(
        runtime.backend.frames[&left],
        crate::Rect {
            x: 0,
            y: 0,
            width: 756,
            height: 982,
        }
    );
    assert_eq!(
        runtime.backend.frames[&right],
        crate::Rect {
            x: 756,
            y: 0,
            width: 756,
            height: 982,
        }
    );
}

#[test]
fn stranded_floating_window_on_visible_workspace_returns_to_built_in_display() {
    let external = external_1080p();
    let built_in = builtin_1512();
    let cancelled = AtomicBool::new(false);
    let dialog = app_window(10, 100);
    let initial = crate::Rect {
        x: 1200,
        y: 300,
        width: 500,
        height: 400,
    };
    let config = Config::parse(
        "version = 1
[[rules]]
bundle_id = 'com.example.Dialog'
action = 'float'",
    )
    .unwrap();
    let backend = FakeBackend {
        frames: HashMap::from([(dialog.clone(), initial)]),
        observation: Some(observation_for("com.example.Dialog", &dialog, initial)),
        ..FakeBackend::default()
    };
    let mut runtime = runtime_with_config(backend, config);
    runtime.cycle(external, &cancelled).unwrap();
    assert_eq!(
        runtime.workspaces.floating_workspace_of(&dialog),
        Some(WorkspaceId::ONE)
    );

    // After disconnect, macOS leaves the floating window off the built-in display.
    let stranded = crate::Rect {
        x: 1511,
        y: 950,
        width: 500,
        height: 400,
    };
    runtime.backend.frames.insert(dialog.clone(), stranded);

    runtime.cycle(built_in.clone(), &cancelled).unwrap();

    assert_eq!(
        runtime.workspaces.floating_workspace_of(&dialog),
        Some(WorkspaceId::ONE)
    );
    let recovered = runtime.backend.frames[&dialog];
    assert_eq!((recovered.width, recovered.height), (500, 400));
    assert!(built_in.main().frame.contains_center(recovered));
}

#[test]
fn off_display_window_being_dragged_with_mouse_down_is_not_yanked_back() {
    let built_in = builtin_1512();
    let cancelled = AtomicBool::new(false);
    let browser = app_window(10, 100);

    let backend = FakeBackend {
        frames: HashMap::from([(browser.clone(), built_in.main().frame)]),
        observation: Some(observation_for(
            "com.example.Browser",
            &browser,
            built_in.main().frame,
        )),
        ..FakeBackend::default()
    };
    let mut runtime = runtime(backend);
    runtime.cycle(built_in.clone(), &cancelled).unwrap();

    // User drags the window until its center is off the display.
    let sliver = bottom_right_sliver();
    runtime.backend.frames.insert(browser.clone(), sliver);
    runtime.backend.mouse_down = true;
    runtime.backend.observation.as_mut().unwrap().mouse_down = true;
    runtime.backend.calls.clear();

    runtime.cycle(built_in.clone(), &cancelled).unwrap();
    assert_eq!(runtime.backend.frames[&browser], sliver);
    assert_eq!(
        runtime.workspaces.workspace_of(&browser),
        Some(WorkspaceId::ONE)
    );
    assert!(
        !runtime.omitted_windows.contains_key(&browser),
        "pointer drag off-display must not burn omission settle time"
    );

    // Once the user releases the mouse, the window snaps back cleanly.
    runtime.backend.mouse_down = false;
    runtime.backend.observation.as_mut().unwrap().mouse_down = false;
    runtime.cycle(built_in.clone(), &cancelled).unwrap();
    assert_eq!(runtime.backend.frames[&browser], built_in.main().frame);
    assert_eq!(
        runtime.workspaces.workspace_of(&browser),
        Some(WorkspaceId::ONE)
    );
}

#[test]
fn ordered_out_window_off_display_is_not_moved_by_stranded_recovery() {
    let external = external_1080p();
    let built_in = builtin_1512();
    let cancelled = AtomicBool::new(false);
    let win = app_window(10, 100);

    let backend = FakeBackend {
        frames: HashMap::from([(win.clone(), external.main().frame)]),
        observation: Some(observation_for(
            "com.example.App",
            &win,
            external.main().frame,
        )),
        ..FakeBackend::default()
    };
    let mut runtime = runtime(backend);
    runtime.cycle(external, &cancelled).unwrap();

    // Window is ordered out (minimized / other Space) while its server frame
    // still sits outside the smaller built-in display.
    let off_display = bottom_right_sliver();
    runtime.backend.frames.remove(&win);
    runtime
        .backend
        .server_frames
        .insert(win.clone(), off_display);
    runtime.backend.ordered_out.insert(win.window_id);
    runtime.backend.calls.clear();

    runtime.cycle(built_in, &cancelled).unwrap();
    assert!(
        !runtime.backend.calls.iter().any(|call| matches!(
            call,
            BackendCall::Apply(applied, _) | BackendCall::SetPosition(applied, _) if applied == &win
        )),
        "ordered-out window must not be moved by stranded visible recovery"
    );
}
