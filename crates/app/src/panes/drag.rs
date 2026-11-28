use crate::layout::{self, Axis, Node};
use crate::tabs;
use crate::workspace::Workspace;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Bounds, Context, DragMoveEvent, Entity, Global, Pixels, Point, SharedString,
    Window, div, px, relative,
};
use nixa_protocol::TermId;
use nixa_ui::metrics::PANE_RADIUS;
use nixa_ui::{Theme, theme};

const EDGE: f32 = 0.3;

#[derive(Debug, Clone, PartialEq)]
pub struct PaneMove {
    pub tab: String,
    pub term: TermId,
    pub label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
    Middle,
}

#[derive(Debug, Clone, PartialEq)]
struct Aim {
    tab: String,
    term: TermId,
    side: Side,
}

#[derive(Default)]
struct Aiming(Option<Aim>);

impl Global for Aiming {}

pub struct Ghost {
    label: String,
}

impl Render for Ghost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        div()
            .px(px(10.))
            .py(px(4.))
            .rounded(px(6.))
            .border_1()
            .border_color(t.accent)
            .bg(t.raised)
            .text_size(px(12.))
            .text_color(t.bright)
            .child(SharedString::from(self.label.clone()))
    }
}

pub fn side_of(x: f32, y: f32) -> Side {
    let edges = [
        (x, Side::Left),
        (1.0 - x, Side::Right),
        (y, Side::Top),
        (1.0 - y, Side::Bottom),
    ];
    let mut best = edges[0];
    for edge in edges {
        if edge.0 < best.0 {
            best = edge;
        }
    }
    if best.0 > EDGE { Side::Middle } else { best.1 }
}

pub fn side_at(bounds: Bounds<Pixels>, at: Point<Pixels>) -> Side {
    let w = f32::from(bounds.size.width).max(1.0);
    let h = f32::from(bounds.size.height).max(1.0);
    side_of(
        f32::from(at.x - bounds.left()) / w,
        f32::from(at.y - bounds.top()) / h,
    )
}

fn swapped(node: &Node, a: &TermId, b: &TermId) -> Node {
    match node {
        Node::Leaf(id) if id == a => Node::Leaf(b.clone()),
        Node::Leaf(id) if id == b => Node::Leaf(a.clone()),
        Node::Leaf(id) => Node::Leaf(id.clone()),
        Node::Split {
            axis,
            children,
            sizes,
        } => Node::Split {
            axis: *axis,
            children: children.iter().map(|c| swapped(c, a, b)).collect(),
            sizes: sizes.clone(),
        },
    }
}

fn pair(leaf: TermId, new: &TermId, before: bool) -> [Node; 2] {
    if before {
        [Node::Leaf(new.clone()), Node::Leaf(leaf)]
    } else {
        [Node::Leaf(leaf), Node::Leaf(new.clone())]
    }
}

fn place(node: Node, target: &TermId, new: &TermId, axis: Axis, before: bool) -> Node {
    match node {
        Node::Leaf(leaf) if &leaf == target => Node::Split {
            axis,
            children: pair(leaf, new, before).into(),
            sizes: Vec::new(),
        },
        Node::Leaf(leaf) => Node::Leaf(leaf),
        Node::Split {
            axis: own,
            children,
            sizes,
        } => {
            let old = layout::shares(&sizes, children.len());
            let mut out = Vec::with_capacity(children.len() + 1);
            let mut out_sizes = Vec::with_capacity(children.len() + 1);
            for (child, share) in children.into_iter().zip(old) {
                match child {
                    Node::Leaf(leaf) if &leaf == target && own == axis => {
                        for half in pair(leaf, new, before) {
                            out.push(half);
                            out_sizes.push(share / 2.0);
                        }
                    }
                    other => {
                        out.push(place(other, target, new, axis, before));
                        out_sizes.push(share);
                    }
                }
            }
            let even = out_sizes.windows(2).all(|w| (w[0] - w[1]).abs() < 1e-4);
            Node::Split {
                axis: own,
                children: out,
                sizes: if even { Vec::new() } else { out_sizes },
            }
        }
    }
}

pub fn moved(root: &Node, moving: &TermId, target: &TermId, side: Side) -> Option<Node> {
    if moving == target || !layout::contains(root, moving) || !layout::contains(root, target) {
        return None;
    }
    let (axis, before) = match side {
        Side::Middle => return Some(swapped(root, moving, target)),
        Side::Left => (Axis::Row, true),
        Side::Right => (Axis::Row, false),
        Side::Top => (Axis::Column, true),
        Side::Bottom => (Axis::Column, false),
    };
    let rest = layout::remove(root.clone(), moving)?;
    Some(place(rest, target, moving, axis, before))
}

pub fn start(
    drag: &PaneMove,
    _at: Point<Pixels>,
    _window: &mut Window,
    cx: &mut App,
) -> Entity<Ghost> {
    cx.set_global(Aiming(None));
    let label = drag.label.clone();
    cx.new(|_| Ghost { label })
}

fn current(cx: &App) -> Option<Aim> {
    cx.try_global::<Aiming>().and_then(|a| a.0.clone())
}

pub fn aim(
    event: &DragMoveEvent<PaneMove>,
    tab: &str,
    term: &TermId,
    cx: &mut Context<Workspace>,
) {
    let drag = event.drag(cx).clone();
    let at = event.event.position;
    let now = current(cx);
    let next = if event.bounds.contains(&at) && drag.tab == tab && drag.term != *term {
        Some(Aim {
            tab: tab.to_string(),
            term: term.clone(),
            side: side_at(event.bounds, at),
        })
    } else if now.as_ref().is_some_and(|a| a.tab == tab && a.term == *term) {
        None
    } else {
        return;
    };
    if now != next {
        cx.set_global(Aiming(next));
        cx.notify();
    }
}

pub fn drop_on(
    ws: &mut Workspace,
    drag: &PaneMove,
    tab: &str,
    term: &TermId,
    cx: &mut Context<Workspace>,
) {
    let aim = current(cx);
    cx.set_global(Aiming(None));
    cx.notify();
    let Some(aim) = aim.filter(|a| a.tab == tab && a.term == *term && drag.tab == tab) else {
        return;
    };
    let Some(open) = tabs::find_mut(&mut ws.tabs, tab) else {
        return;
    };
    let Some(root) = moved(&open.root, &drag.term, term, aim.side) else {
        return;
    };
    open.root = root;
    open.focus = drag.term.clone();
    super::save(ws, cx);
    ws.refocus = true;
}

pub fn overlay(t: &Theme, tab: &str, term: &TermId, cx: &App) -> Option<AnyElement> {
    if !cx.has_active_drag() {
        return None;
    }
    let aim = current(cx).filter(|a| a.tab == tab && a.term == *term)?;
    let base = div()
        .absolute()
        .rounded(px(PANE_RADIUS))
        .border_2()
        .border_color(t.accent)
        .bg(t.accent.opacity(0.18));
    let el = match aim.side {
        Side::Left => base.top_0().bottom_0().left_0().w(relative(0.5)),
        Side::Right => base.top_0().bottom_0().right_0().w(relative(0.5)),
        Side::Top => base.left_0().right_0().top_0().h(relative(0.5)),
        Side::Bottom => base.left_0().right_0().bottom_0().h(relative(0.5)),
        Side::Middle => base.top_0().bottom_0().left_0().right_0(),
    };
    Some(el.into_any_element())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{column, row};

    fn t(n: u64) -> TermId {
        TermId::from_number(n)
    }

    fn leaf(n: u64) -> Node {
        Node::Leaf(t(n))
    }

    fn plain(node: Option<Node>) -> Option<Node> {
        fn strip(node: Node) -> Node {
            match node {
                Node::Leaf(id) => Node::Leaf(id),
                Node::Split { axis, children, .. } => Node::Split {
                    axis,
                    children: children.into_iter().map(strip).collect(),
                    sizes: Vec::new(),
                },
            }
        }
        node.map(strip)
    }

    #[test]
    fn sides_follow_the_nearest_edge() {
        assert_eq!(side_of(0.1, 0.5), Side::Left);
        assert_eq!(side_of(0.9, 0.5), Side::Right);
        assert_eq!(side_of(0.5, 0.05), Side::Top);
        assert_eq!(side_of(0.5, 0.95), Side::Bottom);
        assert_eq!(side_of(0.5, 0.5), Side::Middle);
    }

    #[test]
    fn a_pane_moves_beside_another() {
        let root = row(vec![leaf(1), leaf(2), leaf(3)]);
        let left = moved(&root, &t(3), &t(1), Side::Left);
        assert_eq!(plain(left.clone()), Some(row(vec![leaf(3), leaf(1), leaf(2)])));
        let Some(Node::Split { sizes, .. }) = left else {
            panic!("not a split");
        };
        assert_eq!(sizes, vec![0.25, 0.25, 0.5]);
        assert_eq!(
            moved(&root, &t(1), &t(3), Side::Bottom),
            Some(row(vec![leaf(2), column(vec![leaf(3), leaf(1)])]))
        );
        assert_eq!(
            moved(&root, &t(2), &t(3), Side::Top),
            Some(row(vec![leaf(1), column(vec![leaf(2), leaf(3)])]))
        );
    }

    #[test]
    fn a_nested_pane_leaves_its_split() {
        let root = row(vec![leaf(1), column(vec![leaf(2), leaf(3)])]);
        assert_eq!(
            plain(moved(&root, &t(3), &t(1), Side::Right)),
            Some(row(vec![leaf(1), leaf(3), leaf(2)]))
        );
    }

    #[test]
    fn the_middle_swaps_and_odd_moves_do_nothing() {
        let root = row(vec![leaf(1), column(vec![leaf(2), leaf(3)])]);
        assert_eq!(
            moved(&root, &t(1), &t(3), Side::Middle),
            Some(row(vec![leaf(3), column(vec![leaf(2), leaf(1)])]))
        );
        assert_eq!(moved(&root, &t(1), &t(1), Side::Left), None);
        assert_eq!(moved(&root, &t(9), &t(1), Side::Left), None);
        assert_eq!(moved(&leaf(1), &t(1), &t(2), Side::Left), None);
    }
}
