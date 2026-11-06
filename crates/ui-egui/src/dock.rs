//! Keeping `egui_dock`'s tree and the model's dock in step.
//!
//! The model owns the structure: which panes exist, how they split, which
//! tabs each holds and which is active. `egui_dock` owns the geometry and the
//! gestures: split fractions, tab order within a pane, and the drag and drop
//! that moves tabs around. Before each frame [`DockSync::rebuild`] makes the
//! `egui_dock` tree match the model where their structures differ; after the
//! frame [`DockSync::reconcile`] turns what the user did to the tree into
//! `Command`s, and a second rebuild shows the model's answer.

use std::collections::BTreeSet;

use app::{App, Command, Event};
use egui_dock::{DockState, Node, NodeIndex, Split, SurfaceIndex, TabIndex, Tree};
use model::{Dock, DockTarget, Model, PaneId, Side, TabId};

/// The structure of panes and tabs, as either side sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
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
        /// Left or top.
        first: Box<Shape>,
        /// Right or bottom.
        second: Box<Shape>,
    },
}

impl Shape {
    /// The same structure with each leaf's tabs sorted, so two shapes that
    /// differ only in tab order compare equal.
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
            } => Self::Split {
                horizontal: *horizontal,
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

/// The model's structure.
#[must_use]
pub fn model_shape(model: &Model) -> Shape {
    fn walk(model: &Model, dock: &Dock) -> Shape {
        match dock {
            Dock::Pane(pane) => {
                let pane = model.pane(*pane);
                Shape::Leaf {
                    tabs: pane.map(|p| p.tabs().to_vec()).unwrap_or_default(),
                    active: pane.and_then(model::Pane::active),
                }
            }
            Dock::Split { dir, first, second } => Shape::Split {
                horizontal: *dir == model::SplitDir::Right,
                first: Box::new(walk(model, first)),
                second: Box::new(walk(model, second)),
            },
        }
    }
    walk(model, &model.dock)
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
        Node::Horizontal(_) | Node::Vertical(_) => {
            let horizontal = matches!(&tree[node], Node::Horizontal(_));
            match (
                node_shape(tree, node.left()),
                node_shape(tree, node.right()),
            ) {
                (Some(first), Some(second)) => Some(Shape::Split {
                    horizontal,
                    first: Box::new(first),
                    second: Box::new(second),
                }),
                (one, None) | (None, one) => one,
            }
        }
    }
}

/// The `egui_dock` state and the logic that keeps it true to the model.
pub struct DockSync {
    /// What `DockArea` shows.
    pub state: DockState<TabId>,
}

impl std::fmt::Debug for DockSync {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DockSync")
            .field("shape", &tree_shape(self.state.main_surface()))
            .finish()
    }
}

impl DockSync {
    /// A dock showing the model.
    #[must_use]
    pub fn new(model: &Model) -> Self {
        let mut sync = Self {
            state: DockState::new(Vec::new()),
        };
        sync.rebuild(model);
        sync
    }

    /// Makes the tree match the model. Where a pane's tab set is unchanged
    /// its tab order is kept, and split fractions survive where the split
    /// is still there. Returns whether anything changed.
    pub fn rebuild(&mut self, model: &Model) -> bool {
        let target = model_shape(model);
        if tree_shape(self.state.main_surface()).map(|s| s.normalised())
            == Some(target.normalised())
        {
            return false;
        }
        let old = std::mem::replace(&mut self.state, DockState::new(Vec::new()));
        let old_tree = old.main_surface();
        {
            let tree = self.state.main_surface_mut();
            build(
                tree,
                NodeIndex::root(),
                &target,
                old_tree,
                Some(NodeIndex::root()),
            );
        }
        self.focus_pane(model, model.focused);
        true
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
    /// tab, a changed focus. Then rebuilds the tree from the model.
    pub fn reconcile(&mut self, app: &mut App, events: &mut Vec<Event>) {
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

        // Closed tabs.
        let model_tabs: Vec<TabId> = app
            .model()
            .panes()
            .flat_map(|(_, pane)| pane.tabs().to_vec())
            .collect();
        for tab in model_tabs {
            if !in_tree.contains(&tab) {
                app.handle(Command::CloseTab(tab), events);
            }
        }

        // Each leaf's home pane is the pane of its active tab. A pane claimed
        // by two leaves has been split: the leaf holding the pane's model
        // active tab stays, the other gets a new pane beside a neighbour.
        let mut homes: Vec<(NodeIndex, Option<PaneId>)> = leaves
            .iter()
            .map(|(node, tabs, active)| {
                let pick = active.or_else(|| tabs.first().copied());
                (*node, pick.and_then(|t| app.model().pane_of(t)))
            })
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

        // Focus.
        if let Some(pane) = self.focused_pane(app.model())
            && pane != app.model().focused
        {
            app.handle(Command::FocusPane(pane), events);
        }

        self.rebuild(app.model());
    }

    /// The pane of the sibling subtree of `node` and which side of it `node`
    /// sits on.
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

/// Fills `node` of `tree` with `shape`, reading old order and fractions from
/// `old` at `old_node` where they still apply.
fn build(
    tree: &mut Tree<TabId>,
    node: NodeIndex,
    shape: &Shape,
    old: &Tree<TabId>,
    old_node: Option<NodeIndex>,
) {
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
            first,
            second,
        } => {
            let fraction = old_node
                .filter(|n| n.0 < old.len())
                .and_then(|n| match (&old[n], horizontal) {
                    (Node::Horizontal(s), true) | (Node::Vertical(s), false) => Some(s.fraction),
                    _ => None,
                })
                .unwrap_or(0.5);
            let placeholder = second.first_tab().or_else(|| first.first_tab());
            let Some(placeholder) = placeholder else {
                // No tabs at all on one side: show the other side only.
                let side = if first.first_tab().is_some() {
                    first
                } else {
                    second
                };
                build(tree, node, side, old, old_node);
                return;
            };
            let split = if *horizontal {
                Split::Right
            } else {
                Split::Below
            };
            let [a, b] = tree.split(node, split, fraction, Node::leaf(placeholder));
            let (old_a, old_b) = match old_node {
                Some(n)
                    if n.0 < old.len()
                        && matches!(old[n], Node::Horizontal(_) | Node::Vertical(_)) =>
                {
                    (Some(n.left()), Some(n.right()))
                }
                _ => (None, None),
            };
            build(tree, a, first, old, old_a);
            build(tree, b, second, old, old_b);
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
        let mut dock = DockSync::new(&model);
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
            Some(model_shape(&model).normalised())
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
    }
}
