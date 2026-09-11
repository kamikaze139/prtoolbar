# Tree table popup implementation plan

Goal: Replace misaligned native PR menu labels with a Rust-rendered table and expandable repository/stack tree.

Architecture: eframe/egui owns the native popup and event loop; tray-icon remains the menu bar entry. A channel and egui repaint notification deliver existing background worker events. GitHub fetching and review/stack semantics remain in their existing modules. No webview or unsafe application code.

- [x] Build pure tree rows grouped by repository and stack, preserving activity order across groups and dependency order inside stacks. Test expand/collapse, stable group identities, standalone PRs, partial stacks and empty data.
- [x] Render fixed columns (PR, title, status, reviewers), repository and stack disclosure rows, connector lines, hover details and clickable PR links. Keep column positions independent of text/avatar length. Preserve expansion state on refresh.
- [x] Integrate native popup lifecycle with tray clicks, Escape/close-to-hide, explicit Quit, refresh shortcut and channel-driven repaint. Keep errors and previous successful data visible.
- [x] Verify unit tests, formatting, Clippy, release build, and headless rendering/layout. Launch the updated app and attempt visual inspection; report any inspection limitations. Preserve unrelated workspace edits.

Validation: 70 tests pass, including headless pointer clicks on PR titles and repository names, tree collapse/order, and column boundaries. Formatting, Clippy with warnings denied, and release build pass. Live screenshot confirms table alignment and stack connectors. Native action inspection was interrupted by the computer-use state guard; runtime sampling shows the app is idle in its event loop, not blocked. Popup placement retains its position to avoid incorrect coordinates across mixed-scale displays.
