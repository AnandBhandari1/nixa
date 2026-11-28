pub const SEP: f32 = 28.0;
pub const DUMMY_SEP: f32 = 14.0;
pub const GAP: f32 = 56.0;

struct Graph {
    rank: Vec<i64>,
    dummy: Vec<bool>,
    up: Vec<Vec<usize>>,
    down: Vec<Vec<usize>>,
}

impl Graph {
    fn add(&mut self, rank: i64, dummy: bool) -> usize {
        self.rank.push(rank);
        self.dummy.push(dummy);
        self.up.push(Vec::new());
        self.down.push(Vec::new());
        self.rank.len() - 1
    }
}

fn visit(v: usize, from: &[Vec<(usize, usize)>], state: &mut [u8], back: &mut Vec<bool>) {
    state[v] = 1;
    for &(e, to) in &from[v] {
        match state[to] {
            1 => back[e] = true,
            0 => visit(to, from, state, back),
            _ => {}
        }
    }
    state[v] = 2;
}

pub fn spaced(want: &[f32], w: &[f32], gaps: &[f32]) -> Vec<f32> {
    let mut off = Vec::with_capacity(gaps.len());
    for (i, g) in gaps.iter().enumerate() {
        off.push(if i > 0 { off[i - 1] } else { 0.0 } + g);
    }
    let mut blocks: Vec<(f32, f32, usize)> = Vec::new();
    for (i, v) in want.iter().enumerate() {
        blocks.push((v - off[i], w[i], 1));
        while blocks.len() > 1 && blocks[blocks.len() - 2].0 > blocks[blocks.len() - 1].0 {
            let (v2, w2, n2) = blocks.pop().unwrap_or_default();
            let (v1, w1, n1) = blocks.pop().unwrap_or_default();
            blocks.push(((v1 * w1 + v2 * w2) / (w1 + w2), w1 + w2, n1 + n2));
        }
    }
    let mut out = Vec::with_capacity(want.len());
    for (v, _, n) in blocks {
        for _ in 0..n {
            let i = out.len();
            out.push(v + off[i]);
        }
    }
    out
}

fn crossings(layers: &[Vec<usize>], g: &Graph) -> usize {
    let mut c = 0;
    for r in 0..layers.len().saturating_sub(1) {
        let mut pos = vec![usize::MAX; g.rank.len()];
        for (i, id) in layers[r + 1].iter().enumerate() {
            pos[*id] = i;
        }
        let mut segs: Vec<(i64, i64)> = Vec::new();
        for (i, a) in layers[r].iter().enumerate() {
            for b in &g.down[*a] {
                segs.push((i as i64, pos[*b] as i64));
            }
        }
        for x in 0..segs.len() {
            for y in x + 1..segs.len() {
                if (segs[x].0 - segs[y].0) * (segs[x].1 - segs[y].1) < 0 {
                    c += 1;
                }
            }
        }
    }
    c
}

pub fn layout(n: usize, edges: &[(usize, usize)], size: &[(f32, f32)]) -> Vec<(f32, f32)> {
    if n == 0 {
        return Vec::new();
    }
    let edges: Vec<(usize, usize)> = edges
        .iter()
        .copied()
        .filter(|(a, b)| a != b && *a < n && *b < n)
        .collect();
    let mut from: Vec<Vec<(usize, usize)>> = vec![Vec::new(); n];
    for (e, (a, b)) in edges.iter().enumerate() {
        from[*a].push((e, *b));
    }
    let mut state = vec![0u8; n];
    let mut back = vec![false; edges.len()];
    let mut has_in = vec![false; n];
    for (_, b) in &edges {
        has_in[*b] = true;
    }
    for id in 0..n {
        if !has_in[id] && state[id] == 0 {
            visit(id, &from, &mut state, &mut back);
        }
    }
    for id in 0..n {
        if state[id] == 0 {
            visit(id, &from, &mut state, &mut back);
        }
    }
    let forward: Vec<(usize, usize)> = edges
        .iter()
        .enumerate()
        .filter(|(e, _)| !back[*e])
        .map(|(_, p)| *p)
        .collect();
    let mut ranked: Vec<(usize, usize)> = forward.clone();
    for (e, (a, b)) in edges.iter().enumerate() {
        if back[e] {
            ranked.push((*b, *a));
        }
    }

    let mut rank = vec![0i64; n];
    for _ in 0..n {
        let mut longer = false;
        for (u, v) in &ranked {
            if rank[*v] < rank[*u] + 1 {
                rank[*v] = rank[*u] + 1;
                longer = true;
            }
        }
        if !longer {
            break;
        }
    }
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|id| std::cmp::Reverse(rank[*id]));
    let mut succ: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut preds = vec![0usize; n];
    for (u, v) in &ranked {
        succ[*u].push(*v);
        preds[*v] += 1;
    }
    for _ in 0..4 {
        let mut moved = false;
        for id in &order {
            let outs: Vec<i64> = succ[*id].iter().map(|v| rank[*v]).collect();
            if outs.is_empty() || outs.len() <= preds[*id] {
                continue;
            }
            let to = outs.iter().copied().min().unwrap_or(0) - 1;
            if to > rank[*id] {
                rank[*id] = to;
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
    let top = rank.iter().copied().min().unwrap_or(0);
    for r in rank.iter_mut() {
        *r -= top;
    }
    let ranks = rank.iter().copied().max().unwrap_or(0).max(0) as usize + 1;

    let mut g = Graph {
        rank: Vec::new(),
        dummy: Vec::new(),
        up: Vec::new(),
        down: Vec::new(),
    };
    for r in &rank {
        g.add(*r, false);
    }
    let mut layers: Vec<Vec<usize>> = vec![Vec::new(); ranks];
    for id in 0..n {
        layers[rank[id] as usize].push(id);
    }
    for (a, b) in &forward {
        let mut chain = vec![*a];
        for r in rank[*a] + 1..rank[*b] {
            let id = g.add(r, true);
            layers[r as usize].push(id);
            chain.push(id);
        }
        chain.push(*b);
        for k in 0..chain.len() - 1 {
            g.down[chain[k]].push(chain[k + 1]);
            g.up[chain[k + 1]].push(chain[k]);
        }
    }

    let mut best = layers.clone();
    let mut fewest = crossings(&layers, &g);
    let mut it = 0;
    while it < 16 && fewest > 0 {
        let downward = it % 2 == 0;
        for k in 1..ranks {
            let r = if downward { k } else { ranks - 1 - k };
            let near = if downward { r - 1 } else { r + 1 };
            let mut pos = vec![0f32; g.rank.len()];
            for (i, id) in layers[near].iter().enumerate() {
                pos[*id] = i as f32;
            }
            let bc: Vec<(usize, f32)> = layers[r]
                .iter()
                .enumerate()
                .map(|(i, id)| {
                    let nb = if downward { &g.up[*id] } else { &g.down[*id] };
                    let v = if nb.is_empty() {
                        i as f32
                    } else {
                        nb.iter().map(|x| pos[*x]).sum::<f32>() / nb.len() as f32
                    };
                    (*id, v)
                })
                .collect();
            let mut sorted = bc;
            sorted.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
            layers[r] = sorted.into_iter().map(|(id, _)| id).collect();
        }
        let c = crossings(&layers, &g);
        if c < fewest {
            best = layers.clone();
            fewest = c;
        }
        it += 1;
    }
    let layers = best;

    let along = |id: usize| if g.dummy[id] { 0.0 } else { size[id].0 };
    let across = |id: usize| if g.dummy[id] { 0.0 } else { size[id].1 };
    let apart = |a: usize, b: usize| {
        (across(a) + across(b)) / 2.0
            + if g.dummy[a] || g.dummy[b] {
                DUMMY_SEP
            } else {
                SEP
            }
    };
    let mut main_pos = Vec::with_capacity(ranks);
    let mut acc = 0.0;
    for layer in &layers {
        let ext = layer.iter().map(|id| along(*id)).fold(0.0f32, f32::max);
        main_pos.push(acc + ext / 2.0);
        acc += ext + GAP;
    }
    let total = g.rank.len();
    let mut cross = vec![0f32; total];
    for layer in &layers {
        let mut x = 0.0;
        for (i, id) in layer.iter().enumerate() {
            if i > 0 {
                x += apart(layer[i - 1], *id);
            }
            cross[*id] = x;
        }
        for id in layer {
            cross[*id] -= x / 2.0;
        }
    }
    let weight = |a: usize, b: usize| match (g.dummy[a], g.dummy[b]) {
        (true, true) => 4.0,
        (true, false) | (false, true) => 2.0,
        _ => 1.0,
    };
    let near: Vec<Vec<(usize, f32)>> = (0..total)
        .map(|id| {
            g.up[id]
                .iter()
                .chain(g.down[id].iter())
                .map(|x| (*x, weight(id, *x)))
                .collect()
        })
        .collect();
    let gaps: Vec<Vec<f32>> = layers
        .iter()
        .map(|layer| {
            layer
                .iter()
                .enumerate()
                .map(|(i, id)| if i > 0 { apart(layer[i - 1], *id) } else { 0.0 })
                .collect()
        })
        .collect();
    for it in 0..24 {
        let downward = it % 2 == 0;
        for k in 0..ranks {
            let r = if downward { k } else { ranks - 1 - k };
            let layer = &layers[r];
            let mut want = Vec::with_capacity(layer.len());
            let mut w = Vec::with_capacity(layer.len());
            for id in layer {
                let mut sum = 0.0;
                let mut all = 0.0;
                for (x, wt) in &near[*id] {
                    sum += wt * cross[*x];
                    all += wt;
                }
                want.push(if all > 0.0 { sum / all } else { cross[*id] });
                w.push(if all > 0.0 { all } else { 0.01 });
            }
            let x = spaced(&want, &w, &gaps[r]);
            for (i, id) in layer.iter().enumerate() {
                cross[*id] = x[i];
            }
        }
    }
    for _ in 0..2 {
        for layer in layers.iter().skip(1) {
            for (i, b) in layer.iter().enumerate() {
                if g.up[*b].len() != 1 {
                    continue;
                }
                let a = g.up[*b][0];
                if g.down[a].len() != 1 {
                    continue;
                }
                let mut x = cross[a];
                if i > 0 {
                    x = x.max(cross[layer[i - 1]] + apart(layer[i - 1], *b));
                }
                if i + 1 < layer.len() {
                    x = x.min(cross[layer[i + 1]] - apart(*b, layer[i + 1]));
                }
                cross[*b] = x;
            }
        }
    }
    (0..n)
        .map(|id| (main_pos[g.rank[id] as usize], cross[id]))
        .collect()
}
