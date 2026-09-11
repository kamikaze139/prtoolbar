//! `AppKit` UI and its Objective-C boundary. All UI objects stay on the main thread.
#![allow(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]

use crate::{
    dismissal::{self, ClickTarget},
    events::AppEvent,
    icons, menu,
    model::{CiState, PullRequest, ReviewDecision, ReviewState, Snapshot, Status},
    tree::{self, Row},
    worker::{self, Command},
};
use block2::RcBlock;
use image::{ImageFormat, RgbaImage};
use objc2::{
    AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, ProtocolObject},
    sel,
};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSAutoresizingMaskOptions, NSButton, NSColor,
    NSControlTextEditingDelegate, NSEvent, NSEventMask, NSEventType, NSFont, NSImage, NSImageView,
    NSLineBreakMode, NSOutlineView, NSOutlineViewDataSource, NSOutlineViewDelegate, NSPopover,
    NSPopoverBehavior, NSScrollView, NSTableColumn, NSTableViewColumnAutoresizingStyle,
    NSTableViewStyle, NSTextField, NSView, NSViewController,
};
use objc2_app_kit::{NSApplicationDelegate, NSStatusBar, NSStatusItem, NSVariableStatusItemLength};
use objc2_foundation::{
    NSData, NSInteger, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect, NSRectEdge,
    NSSize, NSString, NSTimer,
};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    io::Cursor,
    ptr::NonNull,
    sync::mpsc::{self, Receiver, Sender},
};

const WIDTH: f64 = 980.0;
const HEIGHT: f64 = 620.0;

#[derive(Debug, Clone)]
struct Item {
    key: Retained<NSString>,
    title: String,
    children: Vec<String>,
    pr: Option<PullRequest>,
}

struct State {
    tray: Retained<NSStatusItem>,
    popover: Retained<NSPopover>,
    table: Retained<NSOutlineView>,
    footer: Retained<NSTextField>,
    heading: Retained<NSTextField>,
    events: Receiver<AppEvent>,
    worker: Sender<Command>,
    snapshot: Snapshot,
    items: HashMap<String, Item>,
    roots: Vec<String>,
    avatars: HashMap<String, Retained<NSImage>>,
}

define_class!(
    // SAFETY: NSObject imposes no subclassing invariants. This class has no Drop.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = RefCell<State>]
    struct Controller;

    // SAFETY: These marker protocols add no implementation requirements.
    unsafe impl NSObjectProtocol for Controller {}
    unsafe impl NSControlTextEditingDelegate for Controller {}

    // SAFETY: AppKit delivers application lifecycle notifications on the main thread.
    unsafe impl NSApplicationDelegate for Controller {
        #[unsafe(method(applicationDidResignActive:))]
        fn resigned_active(&self, _notification: &NSNotification) {
            log::debug!("application deactivated: dismissing popup");
            self.ivars().borrow().popover.close();
        }
    }

    // SAFETY: The data source only supplies retained NSString items held by State.
    unsafe impl NSOutlineViewDataSource for Controller {
        #[unsafe(method(outlineView:numberOfChildrenOfItem:))]
        unsafe fn child_count(&self, _view: &NSOutlineView, item: Option<&AnyObject>) -> NSInteger {
            let state = self.ivars().borrow();
            let children = children(&state, item);
            NSInteger::try_from(children.len()).unwrap_or(0)
        }
        #[unsafe(method_id(outlineView:child:ofItem:))]
        unsafe fn child(&self, _view: &NSOutlineView, index: NSInteger, item: Option<&AnyObject>) -> Retained<AnyObject> {
            let state = self.ivars().borrow();
            let key = &children(&state, item)[usize::try_from(index).expect("AppKit child index")];
            state.items[key].key.clone().into()
        }
        #[unsafe(method(outlineView:isItemExpandable:))]
        unsafe fn expandable(&self, _view: &NSOutlineView, item: &AnyObject) -> bool {
            let state = self.ivars().borrow();
            lookup(&state, item).is_some_and(|item| !item.children.is_empty())
        }
    }

    // SAFETY: Delegate signatures match AppKit; returned views are retained.
    unsafe impl NSOutlineViewDelegate for Controller {
        #[unsafe(method_id(outlineView:viewForTableColumn:item:))]
        unsafe fn cell(&self, _view: &NSOutlineView, column: Option<&NSTableColumn>, item: &AnyObject) -> Option<Retained<NSView>> {
            let state = self.ivars().borrow();
            lookup(&state, item).zip(column).map(|(item, column)| cell_view(self.mtm(), item, &column.identifier().to_string(), column.width(), &state.avatars))
        }
    }

    impl Controller {
        #[unsafe(method(toggle:))]
        fn toggle(&self, _sender: Option<&AnyObject>) {
            let (popover, button) = {
                let state = self.ivars().borrow();
                (state.popover.clone(), state.tray.button(self.mtm()))
            };
            log::debug!("tray action: shown={}", popover.isShown());
            if popover.isShown() {
                popover.close();
                return;
            }
            if let Some(button) = button {
                let screen = button.window().and_then(|window| window.screen());
                let size = screen.map_or(NSSize::new(WIDTH, HEIGHT), |screen| {
                    let frame = screen.visibleFrame();
                    NSSize::new(WIDTH.min(frame.size.width - 32.0), HEIGHT.min(frame.size.height - 40.0))
                });
                popover.setContentSize(size);
                // AppKit positions/clamps the popup and supplies its native material/border.
                popover.showRelativeToRect_ofView_preferredEdge(button.bounds(), &button, NSRectEdge::MinY);
                if let Some(window) = popover.contentViewController().and_then(|vc| vc.view().window()) {
                    window.makeKeyAndOrderFront(None);
                }
                #[allow(deprecated)]
                NSApplication::sharedApplication(self.mtm()).activateIgnoringOtherApps(true);
            }
        }

        #[unsafe(method(tick:))]
        fn tick(&self, _timer: &NSTimer) {
            let mut changed = false;
            loop {
                let event = self.ivars().borrow().events.try_recv();
                let Ok(event) = event else { break };
                let mut state = self.ivars().borrow_mut();
                match event {
                    AppEvent::Loaded { prs, total } => {
                        state.snapshot.loaded(prs, total, jiff::Zoned::now().strftime("%H:%M").to_string());
                    }
                    AppEvent::Failed(error) => state.snapshot.failed(menu::truncate(&error, 100)),
                    AppEvent::Avatars(avatars) => {
                        for (url, pixels) in avatars {
                            if let Some(image) = native_image(&pixels) { state.avatars.insert(url, image); }
                        }
                    }
                }
                changed = true;
            }
            if changed { self.reload(); }
        }

        #[unsafe(method(rowClicked:))]
        fn row_clicked(&self, _sender: &NSOutlineView) {
            let (table, item) = {
                let state = self.ivars().borrow();
                let table = state.table.clone();
                let item = table.itemAtRow(table.clickedRow()).and_then(|item| lookup(&state, &item).cloned());
                (table, item)
            };
            let Some(item) = item else { return };
            if let Some(pr) = item.pr {
                self.ivars().borrow().popover.close();
                if let Err(error) = open::that_detached(&pr.url) { log::warn!("opening PR: {error}"); }
            } else {
                // SAFETY: item.key is retained and belongs to this outline's data source.
                unsafe {
                    if table.isItemExpanded(Some(&item.key)) { table.collapseItem(Some(&item.key)); }
                    else { table.expandItem(Some(&item.key)); }
                }
            }
            // SAFETY: nil sender is accepted by the native deselect action.
            unsafe { table.deselectAll(None); }
        }

        #[unsafe(method(refresh:))]
        fn refresh(&self, _sender: Option<&AnyObject>) {
            let _ = self.ivars().borrow().worker.send(Command::Refresh);
        }
        #[unsafe(method(expandAll:))]
        fn expand_all(&self, _sender: Option<&AnyObject>) {
            let table = self.ivars().borrow().table.clone();
            // SAFETY: nil means all root items; the data source owns the children.
            unsafe { table.expandItem_expandChildren(None, true); }
        }
        #[unsafe(method(collapseAll:))]
        fn collapse_all(&self, _sender: Option<&AnyObject>) {
            let table = self.ivars().borrow().table.clone();
            // SAFETY: nil means all root items.
            unsafe { table.collapseItem_collapseChildren(None, true); }
        }
        #[unsafe(method(quit:))]
        fn quit(&self, _sender: Option<&AnyObject>) {
            // SAFETY: nil sender is accepted by NSApplication's terminate action.
            NSApplication::sharedApplication(self.mtm()).terminate(None);
        }
    }
);

fn lookup<'a>(state: &'a State, item: &AnyObject) -> Option<&'a Item> {
    state
        .items
        .get(&item.downcast_ref::<NSString>()?.to_string())
}

fn children<'a>(state: &'a State, item: Option<&AnyObject>) -> &'a [String] {
    item.map_or(&state.roots, |item| {
        lookup(state, item).map_or(&[], |item| item.children.as_slice())
    })
}

impl Controller {
    fn new(mtm: MainThreadMarker, state: State) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(RefCell::new(state));
        // SAFETY: NSObject's init takes no arguments and initializes this subclass.
        unsafe { msg_send![super(this), init] }
    }

    fn reload(&self) {
        let (table, old_items) = {
            let state = self.ivars().borrow();
            (state.table.clone(), state.items.clone())
        };
        // Capture expansion before replacing data; do not hold a mutable borrow during AppKit callbacks.
        let collapsed: HashSet<_> = old_items
            .iter()
            .filter(|(_, item)| {
                // SAFETY: Every key is an item of the current outline.
                !item.children.is_empty() && !unsafe { table.isItemExpanded(Some(&item.key)) }
            })
            .map(|(key, _)| key.clone())
            .collect();
        {
            let mut state = self.ivars().borrow_mut();
            let (items, roots) = make_items(&state.snapshot.prs, &old_items);
            state.items = items;
            state.roots = roots;
            if let Some(button) = state.tray.button(self.mtm()) {
                button.setToolTip(Some(&NSString::from_str(&menu::tray_tooltip(
                    &state.snapshot,
                ))));
            }
            state.heading.setStringValue(&NSString::from_str(&format!(
                "Pull requests · {} open",
                state.snapshot.total
            )));
            let footer = state.snapshot.error.clone().unwrap_or_else(|| {
                let updated = state
                    .snapshot
                    .updated_at
                    .as_ref()
                    .map_or("Loading…".to_owned(), |at| format!("Updated {at}"));
                if state.snapshot.total > state.snapshot.prs.len() {
                    format!(
                        "{updated} · Showing {} of {}",
                        state.snapshot.prs.len(),
                        state.snapshot.total
                    )
                } else if state.snapshot.prs.is_empty() && state.snapshot.updated_at.is_some() {
                    format!("No open pull requests · {updated}")
                } else {
                    updated
                }
            });
            state.footer.setStringValue(&NSString::from_str(&footer));
            state.footer.setToolTip(Some(&NSString::from_str(&footer)));
            let color = if state.snapshot.error.is_some() {
                NSColor::systemRedColor()
            } else {
                NSColor::secondaryLabelColor()
            };
            state.footer.setTextColor(Some(&color));
        }
        table.reloadData();
        // Expand parents first; preserve collapsed groups across refreshes.
        let keys = {
            let state = self.ivars().borrow();
            let mut keys = Vec::new();
            for root in &state.roots {
                let item = &state.items[root];
                keys.push((root.clone(), item.key.clone()));
                for child in &item.children {
                    if !state.items[child].children.is_empty() {
                        keys.push((child.clone(), state.items[child].key.clone()));
                    }
                }
            }
            keys
        };
        for (key, item) in keys {
            if !collapsed.contains(&key) {
                // SAFETY: These retained items belong to the new data source.
                unsafe {
                    table.expandItem(Some(&item));
                }
            }
        }
    }
}

fn group_key(id: &tree::GroupId) -> String {
    match id {
        tree::GroupId::Repo(repo) => format!("repo:{repo}"),
        tree::GroupId::Stack { repo, number } => format!("repo:{repo}:stack:{number}"),
    }
}

fn make_items(
    prs: &[PullRequest],
    old: &HashMap<String, Item>,
) -> (HashMap<String, Item>, Vec<String>) {
    let mut items: HashMap<String, Item> = HashMap::new();
    let mut roots = Vec::new();
    let mut repo_key = String::new();
    let mut stack_key = String::new();
    for row in tree::rows(prs, &HashSet::new()) {
        let (key, title, parent, pr) = match row {
            Row::Repository { id, repo, count } => {
                repo_key = group_key(&id);
                (repo_key.clone(), format!("{repo}  ({count})"), None, None)
            }
            Row::Stack {
                id,
                number,
                shown,
                total,
                ..
            } => {
                stack_key = group_key(&id);
                (
                    stack_key.clone(),
                    format!("Stack #{number} · {shown} of {total} PRs"),
                    Some(repo_key.clone()),
                    None,
                )
            }
            Row::PullRequest { pr, in_stack, .. } => {
                let title = if pr.is_draft {
                    format!("{} (draft)", pr.title)
                } else {
                    pr.title.clone()
                };
                (
                    pr.url.clone(),
                    title,
                    Some(if in_stack {
                        stack_key.clone()
                    } else {
                        repo_key.clone()
                    }),
                    Some(pr.clone()),
                )
            }
        };
        if let Some(parent) = parent {
            items
                .get_mut(&parent)
                .expect("parent precedes child")
                .children
                .push(key.clone());
        } else {
            roots.push(key.clone());
        }
        let identity = old
            .get(&key)
            .map_or_else(|| NSString::from_str(&key), |item| item.key.clone());
        items.insert(
            key,
            Item {
                key: identity,
                title,
                children: Vec::new(),
                pr,
            },
        );
    }
    (items, roots)
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}

fn label(mtm: MainThreadMarker, text: &str, frame: NSRect) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    label.setFrame(frame);
    label.setFont(Some(&NSFont::systemFontOfSize(12.0)));
    label.setTextColor(Some(&NSColor::labelColor()));
    if let Some(cell) = label.cell() {
        cell.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    }
    label.setToolTip(Some(&NSString::from_str(text)));
    label
}

fn review_text(state: ReviewState) -> (&'static str, Retained<NSColor>) {
    match state {
        ReviewState::Approved => ("✓ Approved", NSColor::systemGreenColor()),
        ReviewState::ChangesRequested => ("✕ Changes requested", NSColor::systemRedColor()),
        ReviewState::Pending => ("◷ Pending", NSColor::systemOrangeColor()),
        ReviewState::Commented => ("· Commented", NSColor::secondaryLabelColor()),
    }
}

fn cell_view(
    mtm: MainThreadMarker,
    item: &Item,
    column: &str,
    width: f64,
    avatars: &HashMap<String, Retained<NSImage>>,
) -> Retained<NSView> {
    let view = NSView::initWithFrame(NSView::alloc(mtm), rect(0.0, 0.0, width, 28.0));
    let Some(pr) = &item.pr else {
        if column == "title" {
            let field = label(mtm, &item.title, rect(0.0, 5.0, width, 20.0));
            field.setFont(Some(&NSFont::boldSystemFontOfSize(12.0)));
            view.addSubview(&field);
        }
        return view;
    };
    if column == "reviewers" && !pr.reviewers.is_empty() {
        let tooltip = pr
            .reviewers
            .iter()
            .map(|reviewer| format!("{}: {}", reviewer.login, review_text(reviewer.state).0))
            .collect::<Vec<_>>()
            .join("\n");
        view.setToolTip(Some(&NSString::from_str(&tooltip)));
        for (i, reviewer) in pr.reviewers.iter().take(4).enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let x = i as f64 * 37.0;
            let image =
                NSImageView::initWithFrame(NSImageView::alloc(mtm), rect(x, 3.0, 22.0, 22.0));
            image.setImage(
                avatars
                    .get(&reviewer.avatar_url)
                    .map(std::ops::Deref::deref),
            );
            image.setToolTip(Some(&NSString::from_str(&format!(
                "{}: {}",
                reviewer.login,
                review_text(reviewer.state).0
            ))));
            view.addSubview(&image);
            let (text, color) = review_text(reviewer.state);
            let badge = label(
                mtm,
                &text.chars().next().unwrap().to_string(),
                rect(x + 22.0, 5.0, 15.0, 18.0),
            );
            badge.setTextColor(Some(&color));
            view.addSubview(&badge);
        }
        if pr.reviewers.len() > 4 {
            view.addSubview(&label(
                mtm,
                &format!("+{}", pr.reviewers.len() - 4),
                rect(148.0, 5.0, 35.0, 18.0),
            ));
        }
        return view;
    }
    let (text, color) = match column {
        "title" => (item.title.clone(), NSColor::labelColor()),
        "pr" => {
            let color = match Status::derive(pr) {
                Status::Ready => NSColor::systemGreenColor(),
                Status::Blocked => NSColor::systemRedColor(),
                Status::Waiting => NSColor::systemOrangeColor(),
                Status::Draft => NSColor::secondaryLabelColor(),
            };
            (format!("● #{}", pr.number), color)
        }
        "checks" => match pr.ci {
            CiState::Success => ("✓ Passed".into(), NSColor::systemGreenColor()),
            CiState::Failure => ("✕ Failing".into(), NSColor::systemRedColor()),
            CiState::Pending => ("◷ Running".into(), NSColor::systemOrangeColor()),
            CiState::None => ("—".into(), NSColor::secondaryLabelColor()),
        },
        "review" => match pr.decision {
            ReviewDecision::Approved => ("✓ Approved".into(), NSColor::systemGreenColor()),
            ReviewDecision::ChangesRequested => {
                ("✕ Changes requested".into(), NSColor::systemRedColor())
            }
            ReviewDecision::Required => ("◷ Review required".into(), NSColor::systemOrangeColor()),
            ReviewDecision::None => ("—".into(), NSColor::secondaryLabelColor()),
        },
        _ => ("—".into(), NSColor::secondaryLabelColor()),
    };
    let field = label(mtm, &text, rect(0.0, 5.0, width, 20.0));
    field.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
    field.setTextColor(Some(&color));
    view.addSubview(&field);
    view
}

/// The menu bar glyph: GitHub's own pull-request mark, drawn by
/// [`icons::menubar_glyph`] as a template image so macOS tints it to match
/// the bar in light mode, dark mode and when the menu is open.
fn menu_bar_image() -> Option<Retained<NSImage>> {
    let image = native_image(&icons::menubar_glyph())?;
    image.setTemplate(true);
    // The bitmap is 36 px so it is pixel-exact at 18 pt on a Retina display.
    image.setSize(NSSize::new(18.0, 18.0));
    Some(image)
}

fn native_image(pixels: &RgbaImage) -> Option<Retained<NSImage>> {
    let mut bytes = Cursor::new(Vec::new());
    pixels.write_to(&mut bytes, ImageFormat::Png).ok()?;
    let data = NSData::with_bytes(bytes.get_ref());
    NSImage::initWithData(NSImage::alloc(), &data)
}

pub fn run() -> anyhow::Result<()> {
    let mtm = MainThreadMarker::new().expect("AppKit must start on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    let popover = NSPopover::new(mtm);
    // Explicit monitoring handles clicks in other apps even if we never became key.
    popover.setBehavior(NSPopoverBehavior::ApplicationDefined);
    popover.setAnimates(true);
    let (content, table, heading, footer) = build_content(mtm, &popover);
    let tray = NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
    let button = tray.button(mtm).expect("native status button");
    if let Some(image) = menu_bar_image() {
        button.setImage(Some(&image));
    }
    button.setToolTip(Some(&NSString::from_str("Pull requests")));
    let (sender, events) = mpsc::channel();
    let interval =
        worker::interval_from_env(std::env::var("PRTOOLBAR_INTERVAL_SECS").ok().as_deref());
    let worker = worker::spawn(move |event| sender.send(event).is_ok(), interval);
    let controller = Controller::new(
        mtm,
        State {
            tray,
            popover: popover.clone(),
            table: table.clone(),
            footer,
            heading,
            events,
            worker,
            snapshot: Snapshot::default(),
            items: HashMap::new(),
            roots: Vec::new(),
            avatars: HashMap::new(),
        },
    );
    app.setDelegate(Some(ProtocolObject::from_ref(&*controller)));
    // SAFETY: Controller lives for the entire application run; selectors below are
    // defined above with their exact AppKit action/data-source signatures.
    unsafe {
        table.setDataSource(Some(ProtocolObject::from_ref(&*controller)));
        table.setDelegate(Some(ProtocolObject::from_ref(&*controller)));
        table.setTarget(Some(&controller));
        table.setAction(Some(sel!(rowClicked:)));
    }
    // SAFETY: Same retained Controller and correctly typed toggle: action.
    unsafe {
        button.setTarget(Some(&controller));
        button.setAction(Some(sel!(toggle:)));
    }
    button.sendActionOn(NSEventMask::LeftMouseUp | NSEventMask::RightMouseUp);
    for (title, selector, x, width, shortcut) in [
        ("Expand all", sel!(expandAll:), WIDTH - 416.0, 95.0, ""),
        ("Collapse all", sel!(collapseAll:), WIDTH - 316.0, 100.0, ""),
        ("Refresh", sel!(refresh:), WIDTH - 211.0, 95.0, "r"),
        ("Quit", sel!(quit:), WIDTH - 111.0, 95.0, "q"),
    ] {
        // SAFETY: All selectors above are one-argument Controller action methods.
        let control = unsafe {
            NSButton::buttonWithTitle_target_action(
                &NSString::from_str(title),
                Some(&controller),
                Some(selector),
                mtm,
            )
        };
        control.setFrame(rect(x, HEIGHT - 40.0, width, 28.0));
        control.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewMinXMargin | NSAutoresizingMaskOptions::ViewMinYMargin,
        );
        control.setKeyEquivalent(&NSString::from_str(shortcut));
        content.addSubview(&control);
    }
    controller.reload();
    // SAFETY: Main-run-loop timer, retained target, exact tick: NSTimer signature.
    let timer = unsafe {
        NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
            0.2,
            &controller,
            sel!(tick:),
            None,
            true,
        )
    };
    timer.setTolerance(0.1);
    let (global, local) = install_monitors(mtm, &popover, &button);
    anyhow::ensure!(
        global.is_some() && local.is_some(),
        "Could not install popup dismissal monitors"
    );
    app.run();
    timer.invalidate();
    for monitor in [global, local].into_iter().flatten() {
        // SAFETY: Tokens came directly from NSEvent's add-monitor methods.
        unsafe {
            NSEvent::removeMonitor(&monitor);
        }
    }
    Ok(())
}

fn build_content(
    mtm: MainThreadMarker,
    popover: &NSPopover,
) -> (
    Retained<NSView>,
    Retained<NSOutlineView>,
    Retained<NSTextField>,
    Retained<NSTextField>,
) {
    let content = NSView::initWithFrame(NSView::alloc(mtm), rect(0.0, 0.0, WIDTH, HEIGHT));
    let vc = NSViewController::new(mtm);
    vc.setView(&content);
    popover.setContentViewController(Some(&vc));
    popover.setContentSize(NSSize::new(WIDTH, HEIGHT));
    let heading = label(mtm, "Pull requests", rect(16.0, HEIGHT - 39.0, 350.0, 24.0));
    heading.setFont(Some(&NSFont::boldSystemFontOfSize(15.0)));
    heading.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinYMargin);
    content.addSubview(&heading);
    let footer = label(mtm, "Loading…", rect(16.0, 10.0, WIDTH - 32.0, 20.0));
    footer.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
    content.addSubview(&footer);
    let scroll = NSScrollView::initWithFrame(
        NSScrollView::alloc(mtm),
        rect(12.0, 38.0, WIDTH - 24.0, HEIGHT - 90.0),
    );
    scroll.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    scroll.setHasVerticalScroller(true);
    scroll.setHasHorizontalScroller(true);
    scroll.setAutohidesScrollers(true);
    scroll.setDrawsBackground(false);
    let table = NSOutlineView::initWithFrame(NSOutlineView::alloc(mtm), scroll.bounds());
    table.setStyle(NSTableViewStyle::FullWidth);
    table.setRowHeight(28.0);
    table.setIntercellSpacing(NSSize::new(10.0, 2.0));
    table.setUsesAlternatingRowBackgroundColors(false);
    table.setBackgroundColor(&NSColor::clearColor());
    table.setColumnAutoresizingStyle(
        NSTableViewColumnAutoresizingStyle::FirstColumnOnlyAutoresizingStyle,
    );
    for (id, title, width) in [
        ("title", "Pull request", 400.0),
        ("pr", "PR", 90.0),
        ("checks", "Checks", 85.0),
        ("review", "Review", 145.0),
        ("reviewers", "Reviewers", 180.0),
    ] {
        let column =
            NSTableColumn::initWithIdentifier(NSTableColumn::alloc(mtm), &NSString::from_str(id));
        column.setTitle(&NSString::from_str(title));
        column.setWidth(width);
        column.setMinWidth(if id == "title" { 200.0 } else { width });
        table.addTableColumn(&column);
        if id == "title" {
            // SAFETY: This column was just added to this outline.
            unsafe {
                table.setOutlineTableColumn(Some(&column));
            }
        }
    }
    scroll.setDocumentView(Some(&table));
    content.addSubview(&scroll);
    (content, table, heading, footer)
}

fn install_monitors(
    mtm: MainThreadMarker,
    popover: &Retained<NSPopover>,
    button: &Retained<objc2_app_kit::NSStatusBarButton>,
) -> (Option<Retained<AnyObject>>, Option<Retained<AnyObject>>) {
    let global_popover = popover.clone();
    let global = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(
        NSEventMask::LeftMouseDown | NSEventMask::RightMouseDown | NSEventMask::OtherMouseDown,
        &RcBlock::new(move |_| {
            if dismissal::dismiss_on_mouse_down(global_popover.isShown(), ClickTarget::Elsewhere) {
                log::debug!("outside click: dismissing popup");
                global_popover.close();
            }
        }),
    );
    let local_popover = popover.clone();
    let local_button = button.clone();
    // SAFETY: AppKit invokes local monitors synchronously on the main thread.
    // The returned pointer is exactly the input event, or nil to consume Escape.
    let local = unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(
            NSEventMask::LeftMouseDown
                | NSEventMask::RightMouseDown
                | NSEventMask::OtherMouseDown
                | NSEventMask::KeyDown,
            &RcBlock::new(move |event: NonNull<NSEvent>| {
                let event_ref = event.as_ref();
                if event_ref.r#type() == NSEventType::KeyDown {
                    if local_popover.isShown() && event_ref.keyCode() == 53 {
                        local_popover.close();
                        return std::ptr::null_mut();
                    }
                    return event.as_ptr();
                }
                let event_window = event_ref.window(mtm);
                let popup_window = local_popover
                    .contentViewController()
                    .and_then(|vc| vc.view().window());
                let target = if event_window.is_some() && event_window == local_button.window() {
                    ClickTarget::TrayIcon
                } else if event_window.is_some() && event_window == popup_window {
                    ClickTarget::Popover
                } else {
                    ClickTarget::Elsewhere
                };
                log::debug!(
                    "local mouse down: target={target:?}, shown={}",
                    local_popover.isShown()
                );
                if dismissal::dismiss_on_mouse_down(local_popover.isShown(), target) {
                    local_popover.close();
                }
                event.as_ptr()
            }),
        )
    };
    (global, local)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::StackMembership;

    fn pr(repo: &str, number: u64, position: u64) -> PullRequest {
        PullRequest {
            repo: repo.into(),
            number,
            title: format!("PR {number}"),
            url: format!("https://github.com/{repo}/pull/{number}"),
            is_draft: false,
            ci: CiState::Success,
            decision: ReviewDecision::Approved,
            reviewers: vec![],
            stack: (position > 0).then_some(StackMembership {
                number: 7,
                position,
                size: 20,
            }),
        }
    }

    #[test]
    fn native_outline_has_real_repo_stack_and_pr_parentage() {
        let tip = pr("a/repo", 3, 3);
        let base = pr("a/repo", 1, 1);
        let standalone = pr("a/repo", 99, 0);
        let other_repo = pr("b/repo", 8, 1);
        let (items, roots) = make_items(
            &[tip.clone(), standalone.clone(), other_repo, base.clone()],
            &HashMap::new(),
        );
        assert_eq!(roots, ["repo:a/repo", "repo:b/repo"]);
        assert_eq!(
            items["repo:a/repo"].children,
            ["repo:a/repo:stack:7", &standalone.url]
        );
        assert_eq!(
            items["repo:a/repo:stack:7"].children,
            [base.url.clone(), tip.url]
        );
        assert_eq!(items["repo:a/repo:stack:7"].title, "Stack #7 · 2 of 20 PRs");
        assert!(items[&base.url].children.is_empty());
        assert_eq!(items[&base.url].pr.as_ref(), Some(&base));
    }

    #[test]
    fn refresh_preserves_native_item_identity_for_expansion_state() {
        let (before, _) = make_items(&[pr("a/repo", 1, 1)], &HashMap::new());
        let (after, _) = make_items(&[pr("a/repo", 2, 2)], &before);
        for key in ["repo:a/repo", "repo:a/repo:stack:7"] {
            assert!(std::ptr::eq(
                &raw const *before[key].key,
                &raw const *after[key].key
            ));
        }
    }
}
