# Spec Delta

## Purpose

Lets the user move a pane back and forth through earlier states of the folder it shows, along a time slider under the toolbar, wherever one or more extensions can say which earlier states exist.

## ADDED Requirements

### Requirement: Timeline offered by extensions
The system SHALL let a Decorator declare that it offers timelines and answer for a location with the live location and a list of points, each with a time, a label, the name of its source and the location that shows the folder at that time. The system SHALL ask for a timeline only off the UI thread and only while the pane's bar is shown, and SHALL keep the merged answer for a location until the location is refreshed, a change its decorators watch arrives for it, or 30 seconds have passed. When several decorators answer for the same location, the system SHALL merge their points into one list sorted by time, each point keeping its source.

#### Scenario: No source
- **WHEN** the pane shows a folder no decorator that offers timelines applies to
- **THEN** neither the timeline bar nor the *Timeline* entry of the view options menu is shown

#### Scenario: Two sources
- **WHEN** two decorators answer for the same folder, one with points on Monday and Thursday, the other with a point on Tuesday
- **THEN** the bar shows three points in the order Monday, Tuesday, Thursday, and each point's tooltip names its source

#### Scenario: Not asked while hidden
- **WHEN** the bar of a pane is hidden and the pane navigates
- **THEN** no decorator is asked for a timeline

#### Scenario: Not asked on the UI thread
- **WHEN** a decorator's answer takes two seconds
- **THEN** the pane lists and responds as usual meanwhile, and the bar shows its reading state until the answer arrives

### Requirement: Timeline bar position and content
The timeline bar SHALL be one row directly below the pane's toolbar and above the file list, the full width of the pane, laid out as the approved design draws it, with an axis that does not move when the pane goes to a past point. It SHALL show a time axis from the oldest point to *Now*, one tick per point at its time, a handle at the current point, and the current point's date, label and source; at *Now* it SHALL show "Now". At a past point it SHALL show that the pane shows the past and is read-only.

#### Scenario: Bar layout
- **WHEN** the bar is shown for a folder with snapshots on Monday, Tuesday and Thursday and the pane is at the live folder
- **THEN** three ticks are placed by time on the axis, the handle is at *Now* at the right end, and the label reads "Now"

#### Scenario: At a past point
- **WHEN** the pane is at the Tuesday snapshot
- **THEN** the bar shows Tuesday's date, the snapshot's name and "ZFS", marked read-only

#### Scenario: Dense ticks
- **WHEN** a timeline has more points than the axis has room for separate ticks
- **THEN** ticks closer than the minimum spacing are drawn as one, and moving the handle still reaches every point

#### Scenario: Hover
- **WHEN** the pointer rests on a tick
- **THEN** a tooltip shows that point's date, time, label and source

### Requirement: Navigating along the timeline
Dragging the handle SHALL snap to the nearest point and take the pane to that point's location once the drag pauses or ends; clicking a tick SHALL go to its point; with the bar focused, Left and Right SHALL step to the previous and next point, Home to the oldest and End to *Now*. Going to *Now* SHALL take the pane to the live location.

#### Scenario: Click a tick
- **WHEN** the user clicks the Tuesday tick while at the live folder `/tank/home/ann/docs`
- **THEN** the pane shows `ann/docs` as of the Tuesday snapshot and the handle sits on Tuesday

#### Scenario: Keyboard
- **WHEN** the bar has focus at the Tuesday point and the user presses Left
- **THEN** the pane shows the Monday point

#### Scenario: Back to now
- **WHEN** the user presses End in the focused bar
- **THEN** the pane shows the live folder and the label reads "Now"

#### Scenario: Folder missing at that time
- **WHEN** the chosen point holds no such folder
- **THEN** the pane shows the nearest existing parent folder at that point and a notice says the folder did not exist then

### Requirement: Scrubbing is one history step
A run of timeline moves SHALL record one history entry, the location before the first move; Back after scrubbing SHALL return there. Any other navigation SHALL end the run.

#### Scenario: Back after scrubbing
- **WHEN** the user at the live folder moves through three points and then presses Back
- **THEN** the pane shows the live folder

### Requirement: Timeline visibility
The bar SHALL be shown when the pane's timeline toggle is on and the location's timeline has points, and always while the pane is at a past point of the timeline it holds; it SHALL never be shown without points, apart from its reading state while the first answer for the location is awaited. A *Timeline* checkbox entry in the toolbar's view options menu and the palette command *Toggle timeline* SHALL switch the toggle for the pane. The entry SHALL be absent where no decorator that offers timelines applies to the location, and disabled with a reason where the timeline was read and has no points. The toolbar SHALL get no new control. The toggle SHALL be saved and restored with the pane.

#### Scenario: Toggle on
- **WHEN** the user ticks *Timeline* in the view options menu in a folder with snapshots
- **THEN** the bar appears, reading at first and then with the points, and the entry is shown ticked

#### Scenario: Toggle in a folder without earlier states
- **WHEN** the user ticks *Timeline* in a dataset without snapshots
- **THEN** no bar remains once the answer arrived, and the entry is disabled with the reason that the folder has no earlier states

#### Scenario: Opened inside a snapshot
- **WHEN** the toggle is on and the pane goes to a folder inside a snapshot through *Browse snapshots*
- **THEN** the bar is shown with the handle at that snapshot

#### Scenario: Switched off in the past
- **WHEN** the pane is at a past point and the user switches the timeline off
- **THEN** the bar stays until the pane is back at *Now*

#### Scenario: Restart
- **WHEN** the toggle is on for a pane and Rick is restarted
- **THEN** that pane shows the bar again
