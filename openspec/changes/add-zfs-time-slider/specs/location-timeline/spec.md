# Spec Delta

## Purpose

Lets the user move a pane back and forth through earlier states of the folder it shows, along a time slider under the toolbar, wherever an extension can say which earlier states exist.

## ADDED Requirements

### Requirement: Timeline offered by an extension
The system SHALL let a Decorator offer a timeline for a location: the live location and an ordered list of points, each with a time, a label and the location that shows the folder at that time. When several decorators offer one, the system SHALL use the first in registration order. A location with no offered timeline SHALL show no timeline controls.

#### Scenario: No source
- **WHEN** the pane shows a folder for which no decorator offers a timeline
- **THEN** neither the timeline bar nor the *Timeline* entry of the view options menu is shown

#### Scenario: Two sources
- **WHEN** two decorators offer a timeline for the same folder
- **THEN** the bar shows the timeline of the one registered first

### Requirement: Timeline bar position and content
The timeline bar SHALL be one row directly below the pane's toolbar and above the file list, the full width of the pane. It SHALL show a time axis from the oldest point to *Now*, one tick per point at its time, a handle at the current point, and the current point's date and label; at *Now* it SHALL show "Now".

#### Scenario: Bar layout
- **WHEN** the bar is shown for a folder with snapshots on Monday, Tuesday and Thursday and the pane is at the live folder
- **THEN** three ticks are placed by time on the axis, the handle is at *Now* at the right end, and the label reads "Now"

#### Scenario: Dense ticks
- **WHEN** a timeline has more points than the axis has room for separate ticks
- **THEN** ticks closer than the minimum spacing are drawn as one, and moving the handle still reaches every point

#### Scenario: Hover
- **WHEN** the pointer rests on a tick
- **THEN** a tooltip shows that point's date, time and label

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
The bar SHALL be shown when the tab's timeline toggle is on and the location has a timeline, and always while the pane is at a past point of a timeline. A *Timeline* checkbox entry in the toolbar's view options menu and the palette command *Toggle timeline* SHALL switch the toggle for the tab; the menu entry SHALL only appear where a timeline exists, and the toolbar SHALL get no new control. The toggle SHALL be saved and restored with the tab.

#### Scenario: Toggle on
- **WHEN** the user ticks *Timeline* in the view options menu in a folder with a timeline
- **THEN** the bar appears and the entry is shown ticked

#### Scenario: Opened inside a snapshot
- **WHEN** the toggle is off and the pane goes to a folder inside a snapshot
- **THEN** the bar is shown

#### Scenario: Restart
- **WHEN** the toggle is on for a tab and Rick is restarted
- **THEN** that tab shows the bar again
