# Native popover implementation plan

**Goal:** One native macOS popup for either tray button, closing on a repeat click, outside click or Escape, preserving the PR table and stack hierarchy.

**Architecture:** Replace the egui/winit window with NSPopover and NSOutlineView. Native controls inherit the system appearance. Application-defined dismissal uses local and global mouse monitors, exempting the anchor button so a mouse-down cannot close and then reopen the popup on mouse-up. Worker/domain logic remains Rust; Objective-C interop is isolated in native.rs.

**Spec:** User request in this session: popup must dismiss outside/on tray and match OS UI.

- [x] Map existing repository/stack rows into native outline items and retain review/avatar tooltips.
- [x] Replace window lifecycle with native popover, status-button action, and event monitors. Preserve startup hidden, refresh, collapse state and error data.
- [x] Add regression coverage for the dismissal race and native hierarchy mapping.
- [x] Build, lint, test, inspect native UI and update usage documentation. Preserve unrelated local edits.

Validation: 51 unit tests cover model/worker behavior, hierarchy mapping, stable native item identity and the outside-click/tray-toggle race. A separate ignored QA executable exercised hidden → open → closed → reopened across native run-loop iterations (allowing AppKit's close animation). CUA inspected the real popover with live PR data and operated Collapse all and Escape. Native event logs confirmed tray toggles and outside mouse dismissal. Application deactivation also closes the popover. The shipping executable contains no QA auto-open behavior.
