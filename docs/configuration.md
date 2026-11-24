# Configuration

Spaceman loads configuration when `spaceman-agent` starts and reloads it automatically whenever the
file changes on disk:

```text
~/Library/Application Support/Spaceman/config.toml
```

Set `SPACEMAN_CONFIG` in the agent's environment to replace that path. A missing file means default
configuration. Invalid configuration at startup is reported to stderr for terminal launches and to
`~/Library/Application Support/Spaceman/agent.log` for app-bundle launches; Spaceman falls back to
defaults and ignores workspace state so it can prioritize restoring parked windows. If an edit saved
while running fails validation, the running agent logs the error and keeps the previous valid
configuration.

Validate a file before replacing or restarting the app:

```sh
spaceman config-check "$HOME/Library/Application Support/Spaceman/config.toml"
```

The complete example is [config.example.toml](../config.example.toml).

## Current Schema

The parser rejects unknown fields. The root must contain `version = 1` and may contain an optional
`[settings]` table plus repeated `[[rules]]` and `[[workspaces]]` tables.

```toml
version = 1

[settings]
hud = true                            # optional (default: true); non-activating Workspace HUD
hud_style = "icons"                   # optional (default: "icons"); or "layouts"
update_channel = "stable"             # optional (default: "stable"); or "dev", a build of every merge
single_window_max_width = 0           # optional (default: 0, off); or 320 through 16384 points

[settings.gaps]                       # optional; every key defaults to 0
inner = 0                             # points between tiled windows, 0 through 48
top = 0                               # outer padding at each screen edge, 0 through 128
bottom = 0
left = 0
right = 0

[[rules]]
bundle_id = "com.example.Application" # optional exact match
title = "Preferences"                 # optional exact match
action = "float"                      # required: "float", "route", or "ignore"

[[rules]]
bundle_id = "com.tinyspeck.slackmacgap"
action = "route"
workspace = 2                         # required for route; forbidden for float and ignore

[[rules]]
bundle_id = "ai.opencode.desktop"
action = "route"
workspace = "D"                       # named workspace A through Z

[[workspaces]]
workspace = 9                         # 1 through 10 or "A" through "Z"; unique
monitor = "main"                      # non-empty selector
```

A rule needs at least one of `bundle_id` or `title`. If both are present, both must match exactly.
Rules run in file order and the first matching rule wins. An unmatched window is managed normally.

- `float` excludes the window from tiling and resizing, preserves its native frame, and keeps it with
  the logical workspace where it was first observed.
- `route` assigns a resizable logical window to workspace 1 through 10 or a configured named
  workspace A through Z. When a `route` rule is added or changed, matching open windows move to the
  target workspace immediately. For newly opened windows afterward, a bundle-level `route` rule
  (without `title`) applies to the application's initial window while secondary windows opened later
  stay on the active workspace; a `title`-matched `route` rule routes every matching window. Movable
  non-resizable windows remain floating on the workspace where first observed. An unchanged rule does
  not repeatedly route existing slots (including manually moved windows or a Ghostty native-tab
  physical replacement).
- `ignore` leaves the window entirely alone: never tiled, never owned by a workspace, never moved or
  parked. Use it for applications that insist on positioning their own windows. Previously managed
  windows (including parked ones) are restored and released when the rule reloads.

During the startup recovery window, an unmatched window of an application that has not yet been seen
in the session may return to a remembered workspace; see
[Remembered Placements](#remembered-placements). Matching `route` and `ignore` rules always win over
memory.

Title matching occurs only in memory. Titles are not serialized into persisted state, snapshots,
`ctl debug`, or the HUD.

## Gaps, Screen Edges, and Single-Window Width

`[settings.gaps]` spaces tiles out. `inner` is the distance between neighboring tiles, in every split
and at every nesting level; `top`, `bottom`, `left`, and `right` pad the tiled area away from each
screen edge (for example, room for a custom menu bar on top). Outer padding gives way before the
tiled area would get smaller than 320 × 240 points. Equal weights still give equal windows with gaps.

`single_window_max_width` caps the width of a window that is alone on its display, either the only
tiled window of a workspace or any window in Fullscreen Layout. The window is centered at that width
inside the padded area; on a display that is narrower, it fills the area as before. It suits wide and
ultrawide displays, where one full-width window is hard to read.

Out-of-range values are rejected with a message naming the key and its range, like any other invalid
configuration. Every key defaults to `0`, so a file without them tiles exactly as before.

Settings › Layout edits the same keys over a live miniature of the result, and `spaceman config`
reads and writes them by dotted key:

```sh
spaceman config set gaps.inner 8
spaceman config set gaps.top 32
spaceman config set single_window_max_width 1680
spaceman config get gaps.inner
```

## Remembered Placements

Window identity in workspace state is tied to a running process, so it cannot survive a reboot or an
application relaunch. Spaceman therefore also remembers, per application bundle identifier, which
workspace each managed window lived on, whether it floated, and its last on-screen frame. It never
records window titles.

During the first 60 seconds after Spaceman starts, when an application bundle is seen for the first
time and owns no windows yet (such as after a reboot or a Spaceman start without valid workspace
state), its windows may claim those remembered placements:
first a window whose frame matches a remembered frame within 8 pixels, then the remaining windows in
creation order. A claimed window joins its remembered workspace (hidden if that workspace is not
visible). Applications launched later during the session, or relaunched after already having been seen,
open on the active workspace instead of recalling historical placements. Windows beyond the remembered
count, windows opened after the recall window closes, and placements on workspaces that are no longer
enabled or managed are adopted on the active workspace. Multi-window applications that
restore windows with identical frames are matched by creation order, which is a best guess.

A changed arrangement replaces an application's memory only after it holds for 10 seconds while the
application keeps running, so the windows closing one by one as an application quits do not erase
the arrangement it should return to. Closing every window leaves the previous memory in place.
Memory is advisory: an unreadable or invalid file is logged and ignored. It holds up to 32 windows for
each of 128 applications, evicting the least recently changed application.

## Display Assignments

Defaults assign workspaces 1-5 to `main` and 6-10 to `secondary`. `monitor` accepts:

- `main`, the display macOS marks as main;
- `secondary`, the first attached non-main display;
- an exact display UUID from `spaceman displays`;
- an exact display name from `spaceman displays`.

```sh
spaceman displays
```

A missing `secondary`, UUID, or name falls back to the main display. Spaceman retains the preferred
assignment as its own active lane; reconnecting that display makes its assigned active workspace
visible again.

Settings › Workspaces offers every connected display: `Main`, `Secondary`, then each further
display, whose assignment it writes as that display's UUID (the same selector
`Option+Shift+Tab` persists). A saved UUID renders back as its display's name, and a saved display
that is unplugged keeps a `Disconnected displays` card so its assignment is never lost.

Named workspaces are opt-in: reference a letter in a route or `[[workspaces]]` entry to enable it.
They default to `main`. An enabled letter registers `Option+letter` to switch and
`Option+Shift+letter` to move the focused window there. The named workspace shortcut takes
precedence over a built-in letter shortcut when they overlap. A named workspace still referenced by
persisted active/previous history, tiled or floating windows, or Fullscreen Layout remains enabled
across config edits until that state no longer refers to it.

## Process Scope

`SPACEMAN_MANAGED_DISPLAY` is an internal process-scope override used by isolated tests. It accepts
`main`, `secondary`, or `both` (the default). It is not a TOML field, and the installed app does not
provide a UI for setting it. A scoped process registers only workspace hotkeys assigned to its managed
main/secondary selectors; normal users should leave it unset.

`SPACEMAN_APPLICATION_SUPPORT` is another internal test override. It replaces the Application Support
directory for the default config path, state, logs, and control files so live harnesses cannot touch
normal recovery data.

## Applying Changes

Edits saved in Spaceman Settings or in an external editor take effect automatically without
restarting: rule changes move, float, tile, or release matching open windows right away; workspace
display assignments, named workspace shortcuts (`Option+A` ... `Option+Z`, registering added letters
and unregistering removed empty ones), `settings.hud`, `settings.hud_style`, `settings.gaps`, and
`settings.single_window_max_width` reload live (Settings › Rules and Layout retile at once; external
edits apply on the next retile); `settings.update_channel` applies at the next update check. If you
ever want a clean restart:

```sh
spaceman ctl pause
spaceman ctl status
spaceman ctl quit
open "/Applications/Spaceman.app"
```

`spaceman ctl restart` safely restores parked windows and exits, but currently does not relaunch the
app by itself. The source installer uses it before replacing the bundle, then opens the new bundle.
