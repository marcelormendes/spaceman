# Known Limitations

Spaceman is an experimental public preview, not a broadly qualified release.

- The downloadable preview is currently arm64-only; x86_64 and universal packaging are not validated.
- The declared macOS 14 minimum lacks a documented validation matrix across supported macOS releases.
- Clean-machine install, permission, upgrade, recovery, and uninstall have not been release-qualified.
- The app depends on private `_AXUIElementGetWindow` behavior and cannot target the Mac App Store.
- Accessibility behavior varies by application; only standard, movable, on-screen windows can be
  workspace-owned. Nonstandard or immovable panels remain global.
- Native-tab identity has no perfect public identifier. Ambiguous transitions intentionally defer
  replacement and may need another cycle.
- After a reboot or application relaunch, remembered placements restore each window's workspace and
  floating state, not its tile position, weight, or nesting within the layout. Without titles,
  windows of a multi-window application that reopen with identical frames are matched in creation
  order and may swap workspaces.
- Configuration supports ordered exact-match `float`/`route`/`ignore` rules and display assignment. It
  intentionally does not support orthogonal named app spaces or regex title matching.
- Configuration changes to rules, display assignments, newly referenced named workspace shortcuts,
  and `[settings]` apply automatically without restarting. `ctl restart` exits safely but does not
  relaunch.
- Spaceman assumes one macOS Space per display. Windows on another Space are adopted into the active
  workspace when that Space is shown, and tiles whose windows are ordered out while it is shown may
  be released.
- `secondary` selects the first non-main display; display ordering and duplicate names may surprise.
- Global shortcuts may collide with other applications or fail on keyboard layouts that cannot
  produce required characters.
- Settings covers Accessibility, Launch at Login, Workspace HUD, app-wide placement rules, workspace
  display assignments, keyboard shortcuts, and Activity diagnostics; title-specific rules are edited in
  `config.toml`. There is no telemetry or analytics.
- The repository still lacks a concise product demo.

Use the isolated live harnesses in [Development](development.md) before changing window-management,
workspace, display, or recovery behavior.
