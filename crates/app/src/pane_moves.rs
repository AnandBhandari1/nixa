use crate::layout::{self, Axis, Node};
use crate::tabs;
use crate::workspace::{EngineState, Workspace};
use gpui::Context;
use nixa_protocol::TermId;

const EPS: f32 = 1e-4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

pub const DIRS: [Dir; 4] = [Dir::Left, Dir::Right, Dir::Up, Dir::Down];

pub fn index(dir: Dir) -> usize {
    DIRS.iter().position(|d| *d == dir).unwrap_or(0)
}

pub fn dir_at(index: usize) -> Dir {
    DIRS.get(index).copied().unwrap_or(Dir::Left)
}

pub fn word(dir: Dir) -> &'static str {
    match dir {
        Dir::Left => "left",
        Dir::Right => "right",
        Dir::Up => "up",
        Dir::Down => "down",
    }
}

pub fn from_word(text: &str) -> Option<Dir> {
    DIRS.into_iter().find(|d| word(*d) == text)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

pub fn rects(node: &Node) -> Vec<(TermId, Rect)> {
    let mut out = Vec::new();
    place(
        node,
        Rect {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
        },
        &mut out,
    );
    out
}

fn place(node: &Node, area: Rect, out: &mut Vec<(TermId, Rect)>) {
    match node {
        Node::Leaf(id) => out.push((id.clone(), area)),
        Node::Split {
            axis,
            children,
            sizes,
        } => {
            let parts = layout::shares(sizes, children.len());
            let mut at = 0.0;
            for (child, share) in children.iter().zip(parts) {
                let part = match axis {
                    Axis::Row => Rect {
                        x: area.x + at * area.w,
                        y: area.y,
                        w: share * area.w,
                        h: area.h,
                    },
                    Axis::Column => Rect {
                        x: area.x,
                        y: area.y + at * area.h,
                        w: area.w,
                        h: share * area.h,
                    },
                };
                at += share;
                place(child, part, out);
            }
        }
    }
}

fn overlap(a0: f32, a1: f32, b0: f32, b1: f32) -> f32 {
    (a1.min(b1) - a0.max(b0)).max(0.0)
}

pub fn toward(node: &Node, from: &TermId, dir: Dir) -> Option<TermId> {
    let all = rects(node);
    let here = all.iter().find(|(id, _)| id == from)?.1;
    all.iter()
        .filter(|(id, _)| id != from)
        .filter_map(|(id, r)| {
            let across_y = overlap(here.y, here.y + here.h, r.y, r.y + r.h);
            let across_x = overlap(here.x, here.x + here.w, r.x, r.x + r.w);
            let (gap, shared) = match dir {
                Dir::Left => (here.x - (r.x + r.w), across_y),
                Dir::Right => (r.x - (here.x + here.w), across_y),
                Dir::Up => (here.y - (r.y + r.h), across_x),
                Dir::Down => (r.y - (here.y + here.h), across_x),
            };
            (gap > -EPS && shared > EPS).then(|| (gap, shared, id.clone()))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0).then(b.1.total_cmp(&a.1)))
        .map(|(_, _, id)| id)
}

pub fn swapped(node: &Node, a: &TermId, b: &TermId) -> Node {
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

fn focused(ws: &Workspace) -> Option<(String, Node, TermId)> {
    let tab = ws.active_tab()?;
    if tabs::zoomed(tab).is_some() {
        return None;
    }
    Some((tab.id.clone(), tab.root.clone(), tab.focus.clone()))
}

pub fn go(ws: &mut Workspace, dir: Dir, cx: &mut Context<Workspace>) {
    let Some((tab, root, focus)) = focused(ws) else {
        return;
    };
    if let Some(next) = toward(&root, &focus, dir) {
        ws.focus_pane(&tab, &next, cx);
    }
}

pub fn swap(ws: &mut Workspace, dir: Dir, cx: &mut Context<Workspace>) {
    let Some((tab, root, focus)) = focused(ws) else {
        return;
    };
    let Some(other) = toward(&root, &focus, dir) else {
        return;
    };
    if let Some(t) = tabs::find_mut(&mut ws.tabs, &tab) {
        t.root = swapped(&root, &focus, &other);
    }
    if ws.engine == EngineState::Up
        && let Err(e) = tabs::persist(&mut ws.tabs, &ws.tabs_path)
    {
        ws.notice = Some(format!("Cannot save the pane layout: {e}"));
    }
    ws.refocus = true;
    cx.notify();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::row;

    fn t(n: u64) -> TermId {
        TermId::from_number(n)
    }

    fn column(children: Vec<Node>) -> Node {
        Node::Split {
            axis: Axis::Column,
            children,
            sizes: Vec::new(),
        }
    }

    fn grid() -> Node {
        row(vec![
            Node::Leaf(t(1)),
            column(vec![Node::Leaf(t(2)), Node::Leaf(t(3))]),
        ])
    }

    #[test]
    fn rects_follow_the_split_sizes() {
        let node = Node::Split {
            axis: Axis::Row,
            children: vec![Node::Leaf(t(1)), Node::Leaf(t(2))],
            sizes: vec![0.25, 0.75],
        };
        let all = rects(&node);
        assert_eq!(all.len(), 2);
        assert!((all[0].1.w - 0.25).abs() < 1e-5);
        assert!((all[1].1.x - 0.25).abs() < 1e-5);
        assert!((all[1].1.w - 0.75).abs() < 1e-5);
    }

    #[test]
    fn focus_moves_to_the_pane_on_that_side() {
        let node = grid();
        assert_eq!(toward(&node, &t(1), Dir::Right), Some(t(2)));
        assert_eq!(toward(&node, &t(3), Dir::Left), Some(t(1)));
        assert_eq!(toward(&node, &t(2), Dir::Down), Some(t(3)));
        assert_eq!(toward(&node, &t(3), Dir::Up), Some(t(2)));
        assert_eq!(toward(&node, &t(1), Dir::Left), None);
        assert_eq!(toward(&node, &t(1), Dir::Up), None);
        assert_eq!(toward(&node, &t(9), Dir::Up), None);
    }

    #[test]
    fn a_lone_pane_has_no_neighbour() {
        let node = Node::Leaf(t(1));
        for dir in DIRS {
            assert_eq!(toward(&node, &t(1), dir), None);
        }
    }

    #[test]
    fn swap_trades_two_leaves_and_keeps_the_shape() {
        let node = grid();
        let out = swapped(&node, &t(1), &t(3));
        assert_eq!(layout::leaves(&out), vec![t(3), t(2), t(1)]);
        assert_eq!(swapped(&out, &t(3), &t(1)), node);
    }

    #[test]
    fn words_go_both_ways() {
        for dir in DIRS {
            assert_eq!(from_word(word(dir)), Some(dir));
            assert_eq!(dir_at(index(dir)), dir);
        }
        assert_eq!(from_word("sideways"), None);
    }
}
