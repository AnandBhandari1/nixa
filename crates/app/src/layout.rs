use nixa_protocol::TermId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const MIN_PANE: f32 = 96.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Axis {
    Row,
    Column,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Node {
    Leaf(TermId),
    Split {
        axis: Axis,
        children: Vec<Node>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        sizes: Vec<f32>,
    },
}

pub fn row(children: Vec<Node>) -> Node {
    Node::Split {
        axis: Axis::Row,
        children,
        sizes: Vec::new(),
    }
}

#[cfg(test)]
pub fn column(children: Vec<Node>) -> Node {
    Node::Split {
        axis: Axis::Column,
        children,
        sizes: Vec::new(),
    }
}

pub fn shares(sizes: &[f32], n: usize) -> Vec<f32> {
    if n == 0 {
        return Vec::new();
    }
    let usable = sizes.len() == n && sizes.iter().all(|s| s.is_finite() && *s > 0.0);
    if !usable {
        return vec![1.0 / n as f32; n];
    }
    let total: f32 = sizes.iter().sum();
    sizes.iter().map(|s| s / total).collect()
}

fn pack(axis: Axis, children: Vec<Node>, sizes: Vec<f32>) -> Option<Node> {
    match children.len() {
        0 => None,
        1 => children.into_iter().next(),
        n => {
            let even = sizes.len() == n && sizes.windows(2).all(|w| (w[0] - w[1]).abs() < 1e-4);
            Some(Node::Split {
                axis,
                sizes: if even { Vec::new() } else { shares(&sizes, n) },
                children,
            })
        }
    }
}

pub fn leaves(node: &Node) -> Vec<TermId> {
    match node {
        Node::Leaf(id) => vec![id.clone()],
        Node::Split { children, .. } => children.iter().flat_map(leaves).collect(),
    }
}

pub fn contains(node: &Node, id: &TermId) -> bool {
    match node {
        Node::Leaf(leaf) => leaf == id,
        Node::Split { children, .. } => children.iter().any(|c| contains(c, id)),
    }
}

pub fn split(node: Node, target: &TermId, new: &TermId, axis: Axis) -> Node {
    match node {
        Node::Leaf(leaf) if &leaf == target => Node::Split {
            axis,
            children: vec![Node::Leaf(leaf), Node::Leaf(new.clone())],
            sizes: Vec::new(),
        },
        Node::Leaf(leaf) => Node::Leaf(leaf),
        Node::Split {
            axis: own,
            children,
            sizes,
        } => {
            let old = shares(&sizes, children.len());
            let mut out = Vec::with_capacity(children.len() + 1);
            let mut out_sizes = Vec::with_capacity(children.len() + 1);
            for (child, share) in children.into_iter().zip(old) {
                match child {
                    Node::Leaf(leaf) if &leaf == target && own == axis => {
                        out.push(Node::Leaf(leaf));
                        out.push(Node::Leaf(new.clone()));
                        out_sizes.push(share / 2.0);
                        out_sizes.push(share / 2.0);
                    }
                    other => {
                        out.push(split(other, target, new, axis));
                        out_sizes.push(share);
                    }
                }
            }
            pack(own, out, out_sizes).unwrap_or(Node::Leaf(new.clone()))
        }
    }
}

pub fn append(node: Node, new: &TermId) -> Node {
    match node {
        Node::Split {
            axis: Axis::Row,
            mut children,
            sizes,
        } => {
            let n = children.len();
            let mut out_sizes: Vec<f32> = shares(&sizes, n)
                .into_iter()
                .map(|s| s * n as f32 / (n + 1) as f32)
                .collect();
            out_sizes.push(1.0 / (n + 1) as f32);
            children.push(Node::Leaf(new.clone()));
            pack(Axis::Row, children, out_sizes).unwrap_or(Node::Leaf(new.clone()))
        }
        other => row(vec![other, Node::Leaf(new.clone())]),
    }
}

fn keep_where(node: Node, keep: &dyn Fn(&TermId) -> bool) -> Option<Node> {
    match node {
        Node::Leaf(leaf) => keep(&leaf).then_some(Node::Leaf(leaf)),
        Node::Split {
            axis,
            children,
            sizes,
        } => {
            let old = shares(&sizes, children.len());
            let mut kept = Vec::new();
            let mut kept_sizes = Vec::new();
            for (child, share) in children.into_iter().zip(old) {
                if let Some(child) = keep_where(child, keep) {
                    kept.push(child);
                    kept_sizes.push(share);
                }
            }
            pack(axis, kept, kept_sizes)
        }
    }
}

pub fn remove(node: Node, target: &TermId) -> Option<Node> {
    keep_where(node, &|id| id != target)
}

pub fn prune(node: Node, keep: &BTreeSet<TermId>) -> Option<Node> {
    keep_where(node, &|id| keep.contains(id))
}

pub fn neighbour(node: &Node, target: &TermId) -> Option<TermId> {
    let all = leaves(node);
    let index = all.iter().position(|id| id == target)?;
    if index > 0 {
        all.get(index - 1).cloned()
    } else {
        all.get(index + 1).cloned()
    }
}

#[cfg(test)]
pub fn at<'a>(node: &'a Node, path: &[usize]) -> Option<&'a Node> {
    match path.split_first() {
        None => Some(node),
        Some((first, rest)) => match node {
            Node::Split { children, .. } => at(children.get(*first)?, rest),
            Node::Leaf(_) => None,
        },
    }
}

fn at_mut<'a>(node: &'a mut Node, path: &[usize]) -> Option<&'a mut Node> {
    match path.split_first() {
        None => Some(node),
        Some((first, rest)) => match node {
            Node::Split { children, .. } => at_mut(children.get_mut(*first)?, rest),
            Node::Leaf(_) => None,
        },
    }
}

pub fn move_edge(
    shares: &[f32],
    edge: usize,
    at: f32,
    length: f32,
    gap: f32,
    min: f32,
) -> Vec<f32> {
    let n = shares.len();
    if edge + 1 >= n || length <= 0.0 {
        return shares.to_vec();
    }
    let room = (length - gap * (n - 1) as f32).max(1.0);
    let start: f32 = shares[..edge].iter().sum::<f32>() * room + gap * edge as f32;
    let pair = shares[edge] + shares[edge + 1];
    let low = (min / room).min(pair / 2.0);
    let high = pair - low;
    let wanted = (at - start - gap / 2.0) / room;
    let first = wanted.clamp(low, high);
    let mut out = shares.to_vec();
    out[edge] = first;
    out[edge + 1] = pair - first;
    out
}

pub fn resize(
    node: &mut Node,
    path: &[usize],
    edge: usize,
    at: f32,
    length: f32,
    gap: f32,
) -> bool {
    let Some(Node::Split {
        children, sizes, ..
    }) = at_mut(node, path)
    else {
        return false;
    };
    let now = shares(sizes, children.len());
    let next = move_edge(&now, edge, at, length, gap, MIN_PANE);
    if next.iter().zip(&now).all(|(a, b)| (a - b).abs() < 1e-5) {
        return false;
    }
    *sizes = next;
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(n: u64) -> TermId {
        TermId::from_number(n)
    }

    fn close(a: &[f32], b: &[f32]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4)
    }

    #[test]
    fn split_and_remove() {
        let root = Node::Leaf(t(1));
        let root = split(root, &t(1), &t(2), Axis::Row);
        let root = split(root, &t(2), &t(3), Axis::Row);
        assert_eq!(leaves(&root), vec![t(1), t(2), t(3)]);
        assert!(matches!(&root, Node::Split { children, .. } if children.len() == 3));
        let root = split(root, &t(1), &t(4), Axis::Column);
        assert_eq!(leaves(&root), vec![t(1), t(4), t(2), t(3)]);
        assert_eq!(neighbour(&root, &t(1)), Some(t(4)));
        assert_eq!(neighbour(&root, &t(2)), Some(t(4)));
        let root = remove(root, &t(4)).unwrap();
        let Node::Split {
            children,
            sizes,
            axis,
        } = &root
        else {
            panic!("not a split");
        };
        assert_eq!(*axis, Axis::Row);
        assert_eq!(
            children,
            &vec![Node::Leaf(t(1)), Node::Leaf(t(2)), Node::Leaf(t(3))]
        );
        assert!(close(&shares(sizes, 3), &[0.5, 0.25, 0.25]));
        let root = remove(remove(root, &t(1)).unwrap(), &t(2)).unwrap();
        assert_eq!(root, Node::Leaf(t(3)));
        assert_eq!(remove(root, &t(3)), None);
    }

    #[test]
    fn prune_keeps_the_shape() {
        let root = split(Node::Leaf(t(1)), &t(1), &t(2), Axis::Column);
        let root = split(root, &t(2), &t(3), Axis::Row);
        let keep: BTreeSet<TermId> = [t(1), t(3)].into_iter().collect();
        assert_eq!(
            prune(root.clone(), &keep),
            Some(column(vec![Node::Leaf(t(1)), Node::Leaf(t(3))]))
        );
        assert_eq!(prune(root, &BTreeSet::new()), None);
    }

    #[test]
    fn append_adds_to_a_row() {
        let root = append(Node::Leaf(t(1)), &t(2));
        let root = append(root, &t(3));
        assert_eq!(leaves(&root), vec![t(1), t(2), t(3)]);
        assert!(contains(&root, &t(3)));
        assert!(!contains(&root, &t(9)));
        assert_eq!(
            root,
            row(vec![Node::Leaf(t(1)), Node::Leaf(t(2)), Node::Leaf(t(3))])
        );
    }

    #[test]
    fn shares_fall_back_to_equal() {
        assert!(close(&shares(&[], 4), &[0.25; 4]));
        assert!(close(&shares(&[1.0, 3.0], 2), &[0.25, 0.75]));
        assert!(close(&shares(&[1.0, f32::NAN], 2), &[0.5, 0.5]));
        assert!(close(&shares(&[1.0, 2.0], 3), &[1.0 / 3.0; 3]));
    }

    #[test]
    fn move_edge_keeps_the_pair_and_the_minimum() {
        let even = [0.5, 0.5];
        let moved = move_edge(&even, 0, 304.0, 1008.0, 8.0, 96.0);
        assert!(close(&moved, &[0.3, 0.7]));
        let tiny = move_edge(&even, 0, 2.0, 1008.0, 8.0, 96.0);
        assert!(close(&tiny, &[0.096, 0.904]));
        let far = move_edge(&even, 0, 5000.0, 1008.0, 8.0, 96.0);
        assert!(close(&far, &[0.904, 0.096]));
        let three = move_edge(&[0.25, 0.25, 0.5], 1, 512.0, 1016.0, 8.0, 50.0);
        assert!(close(&three, &[0.25, 0.25, 0.5]));
        let three = move_edge(&[0.25, 0.25, 0.5], 1, 562.0, 1016.0, 8.0, 50.0);
        assert!(close(&three, &[0.25, 0.3, 0.45]));
        assert!(close(&move_edge(&even, 1, 300.0, 1000.0, 8.0, 96.0), &even));
    }

    #[test]
    fn resize_sets_sizes_inside_a_nested_split() {
        let mut root = row(vec![
            Node::Leaf(t(1)),
            column(vec![Node::Leaf(t(2)), Node::Leaf(t(3))]),
        ]);
        assert!(resize(&mut root, &[1], 0, 154.0, 608.0, 8.0));
        let Some(Node::Split { sizes, .. }) = at(&root, &[1]) else {
            panic!("no split at path");
        };
        assert!(close(sizes, &[0.25, 0.75]));
        assert!(!resize(&mut root, &[1], 0, 154.0, 608.0, 8.0));
        assert!(!resize(&mut root, &[0], 0, 10.0, 600.0, 8.0));
        assert!(!resize(&mut root, &[7], 0, 10.0, 600.0, 8.0));
        let root = split(root, &t(3), &t(4), Axis::Column);
        let Some(Node::Split { sizes, .. }) = at(&root, &[1]) else {
            panic!("no split at path");
        };
        assert!(close(sizes, &[0.25, 0.375, 0.375]));
    }

    #[test]
    fn old_layout_files_still_load() {
        let old = r#"{"Split":{"axis":"Column","children":[{"Leaf":"m1"},{"Leaf":"m2"}]}}"#;
        let node: Node = serde_json::from_str(old).unwrap();
        assert_eq!(node, column(vec![Node::Leaf(t(1)), Node::Leaf(t(2))]));
        let mut sized = node.clone();
        assert!(resize(&mut sized, &[], 0, 100.0, 408.0, 8.0));
        let text = serde_json::to_string(&sized).unwrap();
        assert!(text.contains("sizes"));
        assert_eq!(serde_json::from_str::<Node>(&text).unwrap(), sized);
        assert!(!serde_json::to_string(&node).unwrap().contains("sizes"));
    }
}
