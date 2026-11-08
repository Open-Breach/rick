//! Keeping `egui_dock`'s tree and the model's dock in step, one tree per
//! window.
//!
//! The model owns the structure: which panes exist, how they split, which
//! tabs each holds and which is active. `egui_dock` owns the gestures: the
//! drag and drop that moves tabs around and the divider drags. The display
//! state they produce, tab order within a pane and split fractions, is
//! reported back to the model so the session keeps it (QUESTIONS.md 28 and
//! 67). Before each frame [`DockSync::rebuild`] makes the `egui_dock` tree
//! match the model where their structures differ; after the frame
//! [`DockSync::reconcile`] turns what the user did to the tree into
//! `Command`s, and a second rebuild shows the model's answer.

use std::collections::BTreeSet;

use app::{App, Command, Event};
use egui_dock::{DockState, Node, NodeIndex, Split, SurfaceIndex, TabIndex, Tree};
use model::{Dock, DockTarget, Model, PaneId, Side, TabId, TabKind, WindowId};

/// The structure of panes and tabs, as either side sees it.
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    /// A pane: its tabs in order and the active one.
    Leaf {
        /// Tabs in display order.
        tabs: Vec<TabId>,
        /// The shown tab.
        active: Option<TabId>,
    },
    /// Two parts side by side (`horizontal`) or stacked.
    Split {
        /// Left and right, or above and below.
        horizontal: bool,
        /// How much of the room the first part takes.
        fraction: f32,
        /// Left or top.
        first: Box<Shape>,
        /// Right or bottom.
        second: Box<Shape>,
    },
}

impl Shape {
    /// The same structure with each leaf's tabs sorted and the fractions
    /// dropped, so two shapes that differ only in tab order or divider
    /// positions compare equal.
    fn normalised(&self) -> Self {
        match self {
            Self::Leaf { tabs, active } => {
                let mut tabs = tabs.clone();
                tabs.sort_unstable();
                Self::Leaf {
                    tabs,
                    active: *active,
                }
            }
            Self::Split {
                horizontal,
                first,
                second,
                ..
            } => Self::Split {
                horizontal: *horizontal,
                fraction: 0.0,
                first: Box::new(first.normalised()),
                second: Box::new(second.normalised()),
            },
        }
    }

    /// The first tab in traversal order.
    fn first_tab(&self) -> Option<TabId> {
        match self {
            Self::Leaf { tabs, .. } => tabs.first().copied(),
            Self::Split { first, second, .. } => first.first_tab().or_else(|| second.first_tab()),
        }
    }
}

/// The structure of one window of the model; a leaf for a window the
/// model does not have.
#[must_use]
pub fn model_shape(model: &Model, window: WindowId) -> Shape {
    fn walk(model: &Model, dock: &Dock) -> Shape {
        match dock {
            Dock::Pane(pane) => {
                let pane = model.pane(*pane);
                Shape::Leaf {
                    tabs: pane.map(|p| p.tabs().to_vec()).unwrap_or_default(),
                    active: pane.and_then(model::Pane::active),
                }
            }
            Dock::Split {
                dir,
                fraction,
                first,
                second,
            } => Shape::Split {
                horizontal: *dir == model::SplitDir::Right,
                fraction: *fraction,
                first: Box::new(walk(model, first)),
                second: Box::new(walk(model, second)),
            },
        }
    }
    match model.window(window) {
        Some(w) => walk(model, &w.dock),
        None => Shape::Leaf {
            tabs: Vec::new(),
            active: None,
        },
    }
}

/// The `egui_dock` tree's structure; `None` for an empty tree.
#[must_use]
pub fn tree_shape(tree: &Tree<TabId>) -> Option<Shape> {
    node_shape(tree, NodeIndex::root())
}

fn node_shape(tree: &Tree<TabId>, node: NodeIndex) -> Option<Shape> {
    if node.0 >= tree.len() {
        return None;
    }
    match &tree[node] {
        Node::Empty => None,
        Node::Leaf(leaf) => Some(Shape::Leaf {
            tabs: leaf.tabs.clone(),
            active: leaf.tabs.get(leaf.active.0).copied(),
        }),
        Node::Horizontal(split) | Node::Vertical(split) => {
            let horizontal = matches!(&tree[node], Node::Horizontal(_));
            match (
                node_shape(tree, node.left()),
                node_shape(tree, node.right()),
            ) {
                (Some(first), Some(second)) => Some(Shape::Split {
                    horizontal,
                    fraction: split.fraction,
                    first: Box::new(first),
                    second: Box::new(second),
                }),
                (one, None) | (None, one) => one,
            }
        }
    }
}

/// The `egui_dock` state of one window and the logic that keeps it true to
/// the model.
pub struct DockSync {
    /// What `DockArea` shows.
    pub state: DockState<TabId>,
    /// The window this tree shows.
    pub window: WindowId,
}

impl std::fmt::Debug for DockSync {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DockSync")
            .field("window", &self.window)
            .field("shape", &tree_shape(self.state.main_surface()))
            .finish()
    }
}

impl DockSync {
    /// A dock showing `window` of the model.
    #[must_use]
    pub fn new(model: &Model, window: WindowId) -> Self {
        let mut sync = Self {
            state: DockState::new(Vec::new()),
            window,
        };
        sync.rebuild(model);
        sync
    }

    /// Makes the tree match the model's window. Where a pane's tab set is
    /// unchanged its tab order is kept, the split fractions are the model's,
    /// and a model with another order (a restored session) wins over the
    /// tree's. Returns whether the structure changed.
    pub fn rebuild(&mut self, model: &Model) -> bool {
        let target = model_shape(model, self.window);
        if tree_shape(self.state.main_surface()).map(|s| s.normalised())
            == Some(target.normalised())
        {
            self.apply_fractions(&target);
            return false;
        }
        let old = std::mem::replace(&mut self.state, DockState::new(Vec::new()));
        let old_tree = old.main_surface();
        {
            let tree = self.state.main_surface_mut();
            build(tree, NodeIndex::root(), &target, old_tree);
        }
        let focused = model
            .window(self.window)
            .map_or(model.focused, |w| w.focused);
        self.focus_pane(model, focused);
        true
    }

    /// Puts the model's split fractions into the tree, where the trees have
    /// the same shape.
    fn apply_fractions(&mut self, target: &Shape) {
        fn walk(tree: &mut Tree<TabId>, node: NodeIndex, shape: &Shape) {
            if node.0 >= tree.len() {
                return;
            }
            if let Shape::Split {
                fraction,
                first,
                second,
                ..
            } = shape
                && let Node::Horizontal(split) | Node::Vertical(split) = &mut tree[node]
            {
                if (split.fraction - fraction).abs() > 1e-4 {
                    split.fraction = *fraction;
                }
                walk(tree, node.left(), first);
                walk(tree, node.right(), second);
            }
        }
        walk(self.state.main_surface_mut(), NodeIndex::root(), target);
    }

    /// Replaces the tree's tab order with the model's where the sets agree
    /// (a restored session, a switched layout). Returns whether any changed.
    pub fn take_order(&mut self, model: &Model) -> bool {
        let mut changed = false;
        let tree = self.state.main_surface_mut();
        for node in 0..tree.len() {
            let node = NodeIndex(node);
            let Some(leaf) = tree[node].get_leaf_mut() else {
                continue;
            };
            let Some(pane) = home_of(model, &leaf.tabs) else {
                continue;
            };
            let Some(want) = model.pane(pane).map(|p| p.tabs().to_vec()) else {
                continue;
            };
            if want == leaf.tabs {
                continue;
            }
            let mut have = leaf.tabs.clone();
            let mut wanted = want.clone();
            have.sort_unstable();
            wanted.sort_unstable();
            if have != wanted {
                continue;
            }
            let active = leaf.tabs.get(leaf.active.0).copied();
            leaf.tabs = want;
            if let Some(active) = active
                && let Some(index) = leaf.tabs.iter().position(|t| *t == active)
            {
                leaf.active = TabIndex(index);
            }
            changed = true;
        }
        changed
    }

    /// The window's dock as the tree shows it: the model's panes in the
    /// tree's splits with the tree's fractions; `None` while a leaf holds
    /// tabs the model does not know.
    #[must_use]
    pub fn layout(&self, model: &Model) -> Option<Dock> {
        fn walk(tree: &Tree<TabId>, node: NodeIndex, model: &Model) -> Option<Dock> {
            if node.0 >= tree.len() {
                return None;
            }
            match &tree[node] {
                Node::Empty => None,
                Node::Leaf(leaf) => home_of(model, &leaf.tabs).map(Dock::Pane),
                Node::Horizontal(split) | Node::Vertical(split) => {
                    let horizontal = matches!(&tree[node], Node::Horizontal(_));
                    match (
                        walk(tree, node.left(), model),
                        walk(tree, node.right(), model),
                    ) {
                        (Some(first), Some(second)) => Some(Dock::Split {
                            dir: if horizontal {
                                model::SplitDir::Right
                            } else {
                                model::SplitDir::Down
                            },
                            fraction: split.fraction,
                            first: Box::new(first),
                            second: Box::new(second),
                        }),
                        (one, None) | (None, one) => one,
                    }
                }
            }
        }
        walk(self.state.main_surface(), NodeIndex::root(), model)
    }

    /// The tab a drag out of a tab strip carries right now, if any:
    /// `egui_dock` marks the dragged tab's button id, which is built from the
    /// dock area's id.
    #[must_use]
    pub fn dragged_tab(&self, ctx: &egui::Context, dock_id: egui::Id) -> Option<TabId> {
        let dragged = ctx.dragged_id()?;
        if !ctx.input(|i| i.pointer.is_decidedly_dragging()) {
            return None;
        }
        let tree = self.state.main_surface();
        for (node, tabs) in tree
            .iter()
            .enumerate()
            .filter_map(|(index, node)| node.tabs().map(|tabs| (index, tabs)))
        {
            for (index, tab) in tabs.iter().enumerate() {
                let id = dock_id
                    .with((SurfaceIndex::main(), "surface"))
                    .with((NodeIndex(node), "node"))
                    .with((index, "tab"));
                if id == dragged {
                    return Some(*tab);
                }
            }
        }
        None
    }

    /// Each pane's tab strip in the last frame: the top of its leaf.
    #[must_use]
    pub fn strips(&self, model: &Model, height: f32) -> Vec<(PaneId, egui::Rect)> {
        self.pane_rects(model)
            .into_iter()
            .map(|(pane, rect)| {
                (
                    pane,
                    egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), height)),
                )
            })
            .collect()
    }

    /// Gives the leaf showing `pane` the dock's focus.
    pub fn focus_pane(&mut self, model: &Model, pane: PaneId) {
        let Some(tab) = model
            .pane(pane)
            .and_then(|p| p.active().or_else(|| p.tabs().first().copied()))
        else {
            return;
        };
        if let Some((node, _)) = self.state.main_surface().find_tab(&tab) {
            self.state.main_surface_mut().set_focused_node(node);
        }
    }

    /// The pane whose leaf has the dock's focus.
    #[must_use]
    pub fn focused_pane(&self, model: &Model) -> Option<PaneId> {
        let tree = self.state.main_surface();
        let node = tree.focused_leaf()?;
        let leaf = tree[node].get_leaf()?;
        let tab = leaf.tabs.get(leaf.active.0).or(leaf.tabs.first())?;
        model.pane_of(*tab)
    }

    /// The tabs of `pane` in display order.
    #[must_use]
    pub fn tab_order(&self, model: &Model, pane: PaneId) -> Vec<TabId> {
        let Some(tab) = model
            .pane(pane)
            .and_then(|p| p.active().or_else(|| p.tabs().first().copied()))
        else {
            return Vec::new();
        };
        let tree = self.state.main_surface();
        match tree.find_tab(&tab) {
            Some((node, _)) => tree[node].tabs().map(<[TabId]>::to_vec).unwrap_or_default(),
            None => model
                .pane(pane)
                .map(|p| p.tabs().to_vec())
                .unwrap_or_default(),
        }
    }

    /// Moves `tab` one position within its leaf; display order only.
    pub fn shift_tab(&mut self, tab: TabId, by: isize) {
        let tree = self.state.main_surface_mut();
        let Some((node, index)) = tree.find_tab(&tab) else {
            return;
        };
        let Some(leaf) = tree[node].get_leaf_mut() else {
            return;
        };
        let Some(to) = index.0.checked_add_signed(by) else {
            return;
        };
        if to >= leaf.tabs.len() {
            return;
        }
        leaf.tabs.swap(index.0, to);
        if leaf.active == index {
            leaf.active = TabIndex(to);
        }
    }

    /// The leaf rects by pane, for directional focus.
    #[must_use]
    pub fn pane_rects(&self, model: &Model) -> Vec<(PaneId, egui::Rect)> {
        let tree = self.state.main_surface();
        tree.iter()
            .filter_map(|node| {
                let leaf = node.get_leaf()?;
                let tab = leaf.tabs.get(leaf.active.0).or(leaf.tabs.first())?;
                Some((model.pane_of(*tab)?, leaf.rect))
            })
            .collect()
    }

    /// Turns the user's changes to the tree into commands: closed tabs,
    /// tabs moved into other panes or into new splits, a changed active
    /// tab, a changed focus (only with `may_focus`, when the window has the
    /// system's focus), a changed tab order and changed fractions. Then
    /// rebuilds the tree from the model. `pointer` is where the pointer is,
    /// to tell which pane a drop moved.
    // One pass per kind of change, in the order they depend on each other.
    #[allow(clippy::too_many_lines)]
    pub fn reconcile(
        &mut self,
        app: &mut App,
        events: &mut Vec<Event>,
        pointer: Option<egui::Pos2>,
        may_focus: bool,
    ) {
        let leaves: Vec<(NodeIndex, Vec<TabId>, Option<TabId>)> = {
            let tree = self.state.main_surface();
            (0..tree.len())
                .map(NodeIndex)
                .filter_map(|node| {
                    let leaf = tree[node].get_leaf()?;
                    Some((
                        node,
                        leaf.tabs.clone(),
                        leaf.tabs.get(leaf.active.0).copied(),
                    ))
                })
                .collect()
        };
        let in_tree: BTreeSet<TabId> = leaves
            .iter()
            .flat_map(|(_, tabs, _)| tabs.iter().copied())
            .collect();

        // Closed tabs, among this window's.
        let model_tabs: Vec<TabId> = app
            .model()
            .window(self.window)
            .map(|w| w.dock.panes())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|pane| app.model().pane(pane))
            .flat_map(|pane| pane.tabs().to_vec())
            .collect();
        for tab in model_tabs {
            if !in_tree.contains(&tab) {
                app.handle(Command::CloseTab(tab), events);
            }
        }

        // Each leaf's home pane is the pane that owns most of its tabs, the
        // first tab's pane on a tie. Not the active tab's: egui_dock makes a
        // dropped tab active, and its pane is the one it came from. A pane
        // claimed by two leaves has been split: the leaf holding the pane's
        // model active tab stays, the other gets a new pane beside a
        // neighbour.
        let mut homes: Vec<(NodeIndex, Option<PaneId>)> = leaves
            .iter()
            .map(|(node, tabs, _)| (*node, home_of(app.model(), tabs)))
            .collect();
        for i in 0..homes.len() {
            let Some(home) = homes[i].1 else {
                continue;
            };
            let model_active = app.model().pane(home).and_then(model::Pane::active);
            let claimed_twice = homes
                .iter()
                .enumerate()
                .any(|(j, (_, h))| j != i && *h == Some(home));
            let canonical =
                !claimed_twice || model_active.is_some_and(|a| leaves[i].1.contains(&a));
            if canonical {
                continue;
            }
            // A new pane: put the first tab beside the sibling subtree's pane.
            let node = leaves[i].0;
            let Some((neighbour, side)) = self.neighbour_of(node, &homes, app.model()) else {
                continue;
            };
            let Some(first) = leaves[i].1.first().copied() else {
                continue;
            };
            app.handle(
                Command::MoveTab {
                    tab: first,
                    to: DockTarget::Beside {
                        pane: neighbour,
                        side,
                    },
                },
                events,
            );
            homes[i].1 = app.model().pane_of(first);
        }

        // Tabs that moved into another pane.
        for ((_, tabs, _), (_, home)) in leaves.iter().zip(&homes) {
            let Some(home) = *home else {
                continue;
            };
            for tab in tabs {
                if app.model().pane_of(*tab) != Some(home) {
                    app.handle(
                        Command::MoveTab {
                            tab: *tab,
                            to: DockTarget::Into(home),
                        },
                        events,
                    );
                }
            }
        }

        // Active tabs.
        for ((_, _, active), (_, home)) in leaves.iter().zip(&homes) {
            if let (Some(active), Some(home)) = (active, home)
                && app.model().pane(*home).and_then(model::Pane::active) != Some(*active)
            {
                app.handle(Command::ActivateTab(*active), events);
            }
        }

        self.place_panes(app, events, pointer);

        // Focus. A pane showing the Preview tab never takes it: the preview
        // follows the focus, so the keyboard stays with the explorer.
        if may_focus
            && let Some(pane) = self.focused_pane(app.model())
            && pane != app.model().focused
            && !shows_preview(app.model(), pane)
        {
            app.handle(Command::FocusPane(pane), events);
        }

        // Display state the session keeps: tab order and fractions.
        let panes = app
            .model()
            .window(self.window)
            .map(|w| w.dock.panes())
            .unwrap_or_default();
        for pane in panes {
            let order = self.tab_order(app.model(), pane);
            if app.model().pane(pane).is_some_and(|p| p.tabs() != order) {
                app.handle(Command::TabOrder { pane, order }, events);
            }
        }
        if let Some(dock) = self.layout(app.model())
            && let Some(window) = app.model().window(self.window)
            && window.dock.same_shape(&dock)
            && !same_fractions(&window.dock, &dock)
        {
            let window = self.window;
            app.handle(Command::Layout { window, dock }, events);
        }

        self.rebuild(app.model());
    }

    /// The pane of the sibling subtree of `node` and which side of it `node`
    /// sits on.
    /// Panes moved as a whole: a tab dropped on another pane's edge zone
    /// splits there, and when it was its pane's only tab the tree now has
    /// that pane somewhere else than the model does. Each round moves the
    /// first leaf whose place disagrees beside its sibling in the tree, as a
    /// new pane, until the two agree.
    fn place_panes(&mut self, app: &mut App, events: &mut Vec<Event>, pointer: Option<egui::Pos2>) {
        let leaf_count = self.state.main_surface().num_tabs();
        for _ in 0..=leaf_count {
            let homes: Vec<(NodeIndex, Option<PaneId>)> = {
                let tree = self.state.main_surface();
                (0..tree.len())
                    .map(NodeIndex)
                    .filter_map(|node| {
                        let leaf = tree[node].get_leaf()?;
                        Some((node, home_of(app.model(), &leaf.tabs)))
                    })
                    .collect()
            };
            let Some(tree_panes) = pane_shape(self.state.main_surface(), &homes) else {
                return;
            };
            let Some(model_panes) = model_pane_shape(app.model(), self.window) else {
                return;
            };
            let Some(misplaced) = misplaced_leaf(
                self.state.main_surface(),
                &homes,
                &tree_panes,
                &model_panes,
                pointer,
            ) else {
                return;
            };
            let Some(&(node, _)) = homes.iter().find(|(_, home)| *home == Some(misplaced)) else {
                return;
            };
            let Some(tabs) = self.state.main_surface()[node]
                .tabs()
                .map(<[TabId]>::to_vec)
            else {
                return;
            };
            let Some((neighbour, side)) = self.neighbour_of(node, &homes, app.model()) else {
                return;
            };
            let Some((first, rest)) = tabs.split_first() else {
                return;
            };
            app.handle(
                Command::MoveTab {
                    tab: *first,
                    to: DockTarget::Beside {
                        pane: neighbour,
                        side,
                    },
                },
                events,
            );
            let Some(new_pane) = app.model().pane_of(*first) else {
                return;
            };
            for tab in rest {
                app.handle(
                    Command::MoveTab {
                        tab: *tab,
                        to: DockTarget::Into(new_pane),
                    },
                    events,
                );
            }
        }
    }

    fn neighbour_of(
        &self,
        node: NodeIndex,
        homes: &[(NodeIndex, Option<PaneId>)],
        model: &Model,
    ) -> Option<(PaneId, Side)> {
        let tree = self.state.main_surface();
        let parent = node.parent()?;
        let horizontal = matches!(tree[parent], Node::Horizontal(_));
        let (sibling, side) = if node.is_left() {
            (
                parent.right(),
                if horizontal { Side::Left } else { Side::Top },
            )
        } else {
            (
                parent.left(),
                if horizontal {
                    Side::Right
                } else {
                    Side::Bottom
                },
            )
        };
        let first = node_shape(tree, sibling)?.first_tab()?;
        let pane = homes
            .iter()
            .find_map(|(n, home)| {
                (tree[*n].tabs().is_some_and(|t| t.contains(&first))).then_some(*home)
            })
            .flatten()
            .or_else(|| model.pane_of(first))?;
        Some((pane, side))
    }
}

/// A dock structure over pane ids, to compare where the tree and the model
/// put each pane.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PaneShape {
    Leaf(PaneId),
    Split {
        horizontal: bool,
        first: Box<PaneShape>,
        second: Box<PaneShape>,
    },
}

impl PaneShape {
    fn first_leaf(&self) -> PaneId {
        match self {
            Self::Leaf(pane) => *pane,
            Self::Split { first, .. } => first.first_leaf(),
        }
    }

    fn leaves(&self) -> Vec<PaneId> {
        match self {
            Self::Leaf(pane) => vec![*pane],
            Self::Split { first, second, .. } => {
                let mut leaves = first.leaves();
                leaves.extend(second.leaves());
                leaves
            }
        }
    }

    /// The shape without `pane`, its split collapsed; `None` when nothing is
    /// left.
    fn without(&self, pane: PaneId) -> Option<Self> {
        match self {
            Self::Leaf(p) => (*p != pane).then(|| self.clone()),
            Self::Split {
                horizontal,
                first,
                second,
            } => match (first.without(pane), second.without(pane)) {
                (Some(first), Some(second)) => Some(Self::Split {
                    horizontal: *horizontal,
                    first: Box::new(first),
                    second: Box::new(second),
                }),
                (one, None) | (None, one) => one,
            },
        }
    }
}

/// The pane to move so that the tree and the model agree, or `None` when
/// they do. A dropped pane is one whose removal makes the two shapes equal;
/// when several qualify (two panes that swapped sides, say) the one under
/// the pointer is the dropped one, since `egui_dock`'s drop zones lie on the
/// side the new leaf then takes. Failing both, the first pane where the
/// shapes differ.
fn misplaced_leaf(
    tree: &Tree<TabId>,
    homes: &[(NodeIndex, Option<PaneId>)],
    tree_panes: &PaneShape,
    model_panes: &PaneShape,
    pointer: Option<egui::Pos2>,
) -> Option<PaneId> {
    if tree_panes == model_panes {
        return None;
    }
    let candidates: Vec<PaneId> = tree_panes
        .leaves()
        .into_iter()
        .filter(|pane| tree_panes.without(*pane) == model_panes.without(*pane))
        .collect();
    let under_pointer = pointer.and_then(|pos| {
        candidates.iter().copied().find(|pane| {
            homes.iter().any(|(node, home)| {
                *home == Some(*pane) && tree[*node].rect().is_some_and(|r| r.contains(pos))
            })
        })
    });
    under_pointer
        .or_else(|| candidates.first().copied())
        .or_else(|| divergent_leaf(tree_panes, model_panes))
}

/// The tree's structure with each leaf replaced by its home pane; `None`
/// when the tree holds no pane. A leaf without a home has no tabs the model
/// knows and counts as empty.
fn pane_shape(tree: &Tree<TabId>, homes: &[(NodeIndex, Option<PaneId>)]) -> Option<PaneShape> {
    fn walk(
        tree: &Tree<TabId>,
        node: NodeIndex,
        homes: &[(NodeIndex, Option<PaneId>)],
    ) -> Option<PaneShape> {
        if node.0 >= tree.len() {
            return None;
        }
        match &tree[node] {
            Node::Empty => None,
            Node::Leaf(_) => {
                let home = homes.iter().find(|(n, _)| *n == node)?.1?;
                Some(PaneShape::Leaf(home))
            }
            Node::Horizontal(_) | Node::Vertical(_) => {
                let horizontal = matches!(&tree[node], Node::Horizontal(_));
                match (
                    walk(tree, node.left(), homes),
                    walk(tree, node.right(), homes),
                ) {
                    (Some(first), Some(second)) => Some(PaneShape::Split {
                        horizontal,
                        first: Box::new(first),
                        second: Box::new(second),
                    }),
                    (one, None) | (None, one) => one,
                }
            }
        }
    }
    walk(tree, NodeIndex::root(), homes)
}

/// The model's dock of one window as pane ids.
fn model_pane_shape(model: &Model, window: WindowId) -> Option<PaneShape> {
    fn walk(dock: &Dock) -> PaneShape {
        match dock {
            Dock::Pane(pane) => PaneShape::Leaf(*pane),
            Dock::Split {
                dir, first, second, ..
            } => PaneShape::Split {
                horizontal: *dir == model::SplitDir::Right,
                first: Box::new(walk(first)),
                second: Box::new(walk(second)),
            },
        }
    }
    model.window(window).map(|w| walk(&w.dock))
}

/// Whether two docks of the same shape have the same fractions, give or
/// take a rounding.
fn same_fractions(a: &Dock, b: &Dock) -> bool {
    match (a, b) {
        (Dock::Pane(_), Dock::Pane(_)) => true,
        (
            Dock::Split {
                fraction: fa,
                first: a1,
                second: a2,
                ..
            },
            Dock::Split {
                fraction: fb,
                first: b1,
                second: b2,
                ..
            },
        ) => (fa - fb).abs() <= 1e-4 && same_fractions(a1, b1) && same_fractions(a2, b2),
        _ => false,
    }
}

/// The first pane the tree puts somewhere else than the model, in tree
/// order; `None` when both agree.
fn divergent_leaf(tree: &PaneShape, model: &PaneShape) -> Option<PaneId> {
    match (tree, model) {
        (PaneShape::Leaf(a), PaneShape::Leaf(b)) => (a != b).then_some(*a),
        (
            PaneShape::Split {
                horizontal: h1,
                first: f1,
                second: s1,
            },
            PaneShape::Split {
                horizontal: h2,
                first: f2,
                second: s2,
            },
        ) if h1 == h2 => divergent_leaf(f1, f2).or_else(|| divergent_leaf(s1, s2)),
        (split @ PaneShape::Split { .. }, _) => Some(split.first_leaf()),
        (PaneShape::Leaf(a), PaneShape::Split { .. }) => Some(*a),
    }
}

/// Whether the pane's active tab is the Preview tab.
#[must_use]
pub fn shows_preview(model: &Model, pane: PaneId) -> bool {
    model
        .active_tab(pane)
        .is_some_and(|(_, tab)| tab.kind == TabKind::Preview)
}

/// The pane owning most of `tabs`; the first tab's pane on a tie.
fn home_of(model: &Model, tabs: &[TabId]) -> Option<PaneId> {
    let mut counts: Vec<(PaneId, usize)> = Vec::new();
    for tab in tabs {
        let Some(pane) = model.pane_of(*tab) else {
            continue;
        };
        match counts.iter_mut().find(|(p, _)| *p == pane) {
            Some((_, n)) => *n += 1,
            None => counts.push((pane, 1)),
        }
    }
    // `max_by_key` returns the last maximum; the first pane seen wins a tie.
    let best = counts.iter().map(|(_, n)| *n).max()?;
    counts.iter().find(|(_, n)| *n == best).map(|(p, _)| *p)
}

/// Fills `node` of `tree` with `shape`, reading the old tab order from `old`
/// where a leaf's tab set is unchanged; the fractions are the shape's.
fn build(tree: &mut Tree<TabId>, node: NodeIndex, shape: &Shape, old: &Tree<TabId>) {
    match shape {
        Shape::Leaf { tabs, active } => {
            let ordered = old_order(old, tabs);
            let active = active
                .and_then(|a| ordered.iter().position(|t| *t == a))
                .unwrap_or(0);
            if let Some(leaf) = tree[node].get_leaf_mut() {
                leaf.tabs = ordered;
                leaf.active = TabIndex(active);
            } else {
                tree[node] = Node::leaf_with(ordered);
                if let Some(leaf) = tree[node].get_leaf_mut() {
                    leaf.active = TabIndex(active);
                }
            }
        }
        Shape::Split {
            horizontal,
            fraction,
            first,
            second,
        } => {
            let placeholder = second.first_tab().or_else(|| first.first_tab());
            let Some(placeholder) = placeholder else {
                // No tabs at all on one side: show the other side only.
                let side = if first.first_tab().is_some() {
                    first
                } else {
                    second
                };
                build(tree, node, side, old);
                return;
            };
            let split = if *horizontal {
                Split::Right
            } else {
                Split::Below
            };
            let fraction = if fraction.is_finite() {
                fraction.clamp(0.05, 0.95)
            } else {
                0.5
            };
            let [a, b] = tree.split(node, split, fraction, Node::leaf(placeholder));
            build(tree, a, first, old);
            build(tree, b, second, old);
        }
    }
}

/// `tabs` in the order an old leaf with the same set showed them, else as
/// given.
fn old_order(old: &Tree<TabId>, tabs: &[TabId]) -> Vec<TabId> {
    let wanted: BTreeSet<TabId> = tabs.iter().copied().collect();
    for node in old.iter() {
        if let Some(existing) = node.tabs() {
            let set: BTreeSet<TabId> = existing.iter().copied().collect();
            if set == wanted {
                return existing.to_vec();
            }
        }
    }
    tabs.to_vec()
}

/// The main surface, for `DockArea` paths.
#[must_use]
pub fn main_surface() -> SurfaceIndex {
    SurfaceIndex::main()
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::{Settings, SplitDir};
    use types::Location;

    fn loc(path: &str) -> Location {
        Location::new(types::ProviderId("t"), path)
    }

    #[test]
    fn the_tree_follows_splits_and_tabs_in_the_model() {
        let mut model = Model::new(Settings::default(), loc("/a"));
        let pane = model.focused;
        let window = model.focused_window;
        let mut dock = DockSync::new(&model, window);
        assert_eq!(
            tree_shape(dock.state.main_surface()),
            Some(Shape::Leaf {
                tabs: vec![TabId(1)],
                active: Some(TabId(1))
            })
        );

        let (_, second) = model.new_tab(Some(pane), loc("/b"));
        let (right, _) = model
            .split(pane, SplitDir::Right)
            .unwrap_or_else(|| panic!("split"));
        let (_, below) = model
            .split(right, SplitDir::Down)
            .unwrap_or_else(|| panic!("split"));
        assert!(dock.rebuild(&model));
        assert_eq!(
            tree_shape(dock.state.main_surface()).map(|s| s.normalised()),
            Some(model_shape(&model, window).normalised())
        );
        assert!(!dock.rebuild(&model), "nothing to do when in step");
        assert_eq!(dock.tab_order(&model, pane), [TabId(1), second]);

        // Display order is the dock's business and survives a rebuild.
        dock.shift_tab(second, -1);
        assert_eq!(dock.tab_order(&model, pane), [second, TabId(1)]);
        model.close_tab(below);
        assert!(dock.rebuild(&model));
        assert_eq!(dock.tab_order(&model, pane), [second, TabId(1)]);
        assert_eq!(dock.pane_rects(&model).len(), 2);

        // The model's order wins when it changes (a restored session), and
        // the tree reports its own layout with the model's panes.
        assert!(model.set_tab_order(pane, vec![TabId(1), second]));
        assert!(dock.take_order(&model));
        assert_eq!(dock.tab_order(&model, pane), [TabId(1), second]);
        assert!(!dock.take_order(&model));
        let layout = dock.layout(&model).unwrap_or_else(|| panic!("layout"));
        assert!(layout.same_shape(model.dock()));
        assert!(same_fractions(&layout, model.dock()));
        // A fraction set in the model reaches the tree.
        let mut moved = model.dock().clone();
        if let Dock::Split { fraction, .. } = &mut moved {
            *fraction = 0.25;
        }
        assert!(model.set_layout(window, moved));
        assert!(!dock.rebuild(&model), "same structure");
        let layout = dock.layout(&model).unwrap_or_else(|| panic!("layout"));
        assert!(same_fractions(&layout, model.dock()));
    }
}
