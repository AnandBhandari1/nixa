use super::git::{self, Done, Kind};

pub const NO_NEWLINE: &str = "\\ No newline at end of file";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Stage,
    Unstage,
    Discard,
}

impl Mode {
    pub fn fail_title(self) -> &'static str {
        match self {
            Mode::Stage => "Could not stage this",
            Mode::Unstage => "Could not unstage this",
            Mode::Discard => "Could not discard this",
        }
    }

    fn apply_flags(self) -> &'static [&'static str] {
        match self {
            Mode::Stage => &["--cached"],
            Mode::Unstage => &["--cached", "-R"],
            Mode::Discard => &["-R"],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HunkHead {
    pub old_start: u32,
    pub old_count: u32,
    pub new_start: u32,
    pub new_count: u32,
}

fn range(text: &str) -> Option<(u32, u32)> {
    match text.split_once(',') {
        Some((a, b)) => Some((a.parse().ok()?, b.parse().ok()?)),
        None => Some((text.parse().ok()?, 1)),
    }
}

pub fn parse_head(line: &str) -> Option<(HunkHead, String)> {
    let rest = line.strip_prefix("@@ -")?;
    let (old, rest) = rest.split_once(" +")?;
    let (new, heading) = rest.split_once(" @@")?;
    let (old_start, old_count) = range(old)?;
    let (new_start, new_count) = range(new)?;
    Some((
        HunkHead {
            old_start,
            old_count,
            new_start,
            new_count,
        },
        heading.to_string(),
    ))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BodyLine {
    pub at: usize,
    pub sign: char,
    pub text: String,
    pub eol: bool,
}

pub fn body(lines: &[String]) -> Vec<BodyLine> {
    let mut out: Vec<BodyLine> = Vec::new();
    for (at, line) in lines.iter().enumerate().skip(1) {
        if line.starts_with('\\') {
            if let Some(last) = out.last_mut() {
                last.eol = false;
            }
            continue;
        }
        let sign = line.chars().next().unwrap_or(' ');
        if sign != ' ' && sign != '+' && sign != '-' {
            break;
        }
        out.push(BodyLine {
            at,
            sign,
            text: line.get(1..).unwrap_or("").to_string(),
            eol: true,
        });
    }
    out
}

pub struct Split {
    pub header: Vec<String>,
    pub hunks: Vec<Vec<String>>,
}

pub fn split_hunks(text: &str) -> Split {
    let text = text.strip_suffix('\n').unwrap_or(text);
    let lines: Vec<&str> = text.split('\n').collect();
    let Some(first) = lines.iter().position(|l| l.starts_with("@@")) else {
        return Split {
            header: Vec::new(),
            hunks: Vec::new(),
        };
    };
    let mut hunks: Vec<Vec<String>> = Vec::new();
    for line in &lines[first..] {
        if line.starts_with("@@") {
            hunks.push(vec![line.to_string()]);
        } else if let Some(h) = hunks.last_mut() {
            h.push(line.to_string());
        }
    }
    Split {
        header: lines[..first].iter().map(|s| s.to_string()).collect(),
        hunks,
    }
}

pub fn change_indexes(lines: &[String]) -> Vec<usize> {
    body(lines).into_iter().filter(|l| l.sign != ' ').map(|l| l.at).collect()
}

pub fn complete(lines: &[String]) -> bool {
    let Some((head, _)) = lines.first().and_then(|l| parse_head(l)) else {
        return false;
    };
    let mut old = 0;
    let mut now = 0;
    for l in body(lines) {
        if l.sign != '+' {
            old += 1;
        }
        if l.sign != '-' {
            now += 1;
        }
    }
    old == head.old_count && now == head.new_count
}

fn sides(sign: char) -> &'static [char] {
    match sign {
        ' ' => &['o', 'n'],
        '-' => &['o'],
        _ => &['n'],
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OutLine {
    sign: char,
    text: String,
    eol: bool,
}

fn ends_valid(out: &[OutLine]) -> bool {
    for (i, l) in out.iter().enumerate() {
        if l.eol {
            continue;
        }
        let mine = sides(l.sign);
        if out[i + 1..]
            .iter()
            .any(|o| sides(o.sign).iter().any(|s| mine.contains(s)))
        {
            return false;
        }
    }
    true
}

fn partner_of(body: &[BodyLine], index: usize) -> Option<usize> {
    let mut s = index;
    let mut e = index;
    while s > 0 && body[s - 1].sign != ' ' {
        s -= 1;
    }
    while e + 1 < body.len() && body[e + 1].sign != ' ' {
        e += 1;
    }
    let minus: Vec<usize> = (s..=e).filter(|&i| body[i].sign == '-').collect();
    let plus: Vec<usize> = (s..=e).filter(|&i| body[i].sign == '+').collect();
    let k = minus.iter().position(|&i| i == index)?;
    plus.get(k).copied()
}

fn path_line(line: &str, prefix: &str, side: char) -> String {
    let rest = line.get(4..).unwrap_or("");
    let quoted = rest.starts_with('"');
    let inner = if quoted { &rest[1..] } else { rest };
    let tail = inner.get(1..).unwrap_or("");
    format!("{prefix} {}{side}{tail}", if quoted { "\"" } else { "" })
}

pub fn file_header(header: &[String], old_count: usize, new_count: usize) -> Vec<String> {
    let lines: Vec<String> = header.iter().filter(|l| !l.starts_with("index ")).cloned().collect();
    let is_new = lines.iter().any(|l| l.starts_with("new file mode "));
    let is_gone = lines.iter().any(|l| l.starts_with("deleted file mode "));
    if is_new && old_count > 0 {
        let plus = lines.iter().find(|l| l.starts_with("+++ ")).cloned();
        return lines
            .into_iter()
            .filter(|l| !l.starts_with("new file mode "))
            .map(|l| match (&plus, l.starts_with("--- ")) {
                (Some(p), true) => path_line(p, "---", 'a'),
                _ => l,
            })
            .collect();
    }
    if is_gone && new_count > 0 {
        let minus = lines.iter().find(|l| l.starts_with("--- ")).cloned();
        return lines
            .into_iter()
            .filter(|l| !l.starts_with("deleted file mode "))
            .map(|l| match (&minus, l.starts_with("+++ ")) {
                (Some(m), true) => path_line(m, "+++", 'b'),
                _ => l,
            })
            .collect();
    }
    lines
}

pub fn line_patch(header: &[String], lines: &[String], picked: &[usize], mode: Mode) -> Result<String, String> {
    let Some((head, heading)) = lines.first().and_then(|l| parse_head(l)) else {
        return Err("This part of the diff is not complete. Look at the diff again.".into());
    };
    if !complete(lines) {
        return Err("This part of the diff is not complete. Look at the diff again.".into());
    }
    let body = body(lines);
    let mut chosen: Vec<bool> = body
        .iter()
        .map(|l| l.sign != ' ' && picked.contains(&l.at))
        .collect();
    if !chosen.iter().any(|c| *c) {
        return Err("Pick one or more added or removed lines first.".into());
    }
    let forward = mode == Mode::Stage;
    let mut extra: Option<(usize, OutLine)> = None;
    if forward
        && let Some(last) = body.iter().position(|l| l.sign == '-' && !l.eol) {
            let later_plus = body[last + 1..]
                .iter()
                .enumerate()
                .any(|(k, l)| l.sign == '+' && chosen[last + 1 + k]);
            if !chosen[last] && later_plus {
                chosen[last] = true;
                match partner_of(&body, last) {
                    Some(p) if body[p].text == body[last].text => chosen[p] = true,
                    _ => {
                        extra = Some((
                            last,
                            OutLine {
                                sign: '+',
                                text: body[last].text.clone(),
                                eol: true,
                            },
                        ))
                    }
                }
            }
        }
    let mut out: Vec<OutLine> = Vec::new();
    for (i, l) in body.iter().enumerate() {
        if l.sign == ' ' || chosen[i] {
            out.push(OutLine {
                sign: l.sign,
                text: l.text.clone(),
                eol: l.eol,
            });
        } else if (forward && l.sign == '-') || (!forward && l.sign == '+') {
            out.push(OutLine {
                sign: ' ',
                text: l.text.clone(),
                eol: l.eol,
            });
        }
        if let Some((at, line)) = &extra
            && *at == i {
                out.push(line.clone());
            }
    }
    if !ends_valid(&out) {
        return Err("These lines touch the end of a file that has no newline at the end. Pick the lines after them too, or use the hunk buttons.".into());
    }
    let old_count = out.iter().filter(|l| l.sign != '+').count();
    let new_count = out.iter().filter(|l| l.sign != '-').count();
    let start = |s: u32, n: usize| if n > 0 && s == 0 { 1 } else { s };
    let top = format!(
        "@@ -{},{old_count} +{},{new_count} @@{heading}",
        start(head.old_start, old_count),
        start(head.new_start, new_count)
    );
    let mut text: Vec<String> = Vec::new();
    for l in &out {
        text.push(format!("{}{}", l.sign, l.text));
        if !l.eol {
            text.push(NO_NEWLINE.to_string());
        }
    }
    Ok(format!(
        "{}\n{top}\n{}\n",
        file_header(header, old_count, new_count).join("\n"),
        text.join("\n")
    ))
}

pub fn apply(root: &str, patch: &str, mode: Mode) -> Done {
    if patch.is_empty() || patch.contains('\0') {
        return git::refusal(Kind::Invalid, mode.fail_title(), "There is no text change to apply.");
    }
    let mut args = vec!["apply"];
    args.extend_from_slice(mode.apply_flags());
    args.extend(["--recount", "--whitespace=nowarn", "-"]);
    let out = git::run_in(root, &args, patch);
    if out.ok() {
        return git::success("Done");
    }
    let mut d = git::failure(&out.said());
    if d.kind == Kind::Other {
        d.kind = Kind::Apply;
        d.title = mode.fail_title().into();
        d.hint = "The file may have changed. Look at the diff again, then try once more.".into();
    }
    d
}
