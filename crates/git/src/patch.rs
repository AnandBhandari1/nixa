use crate::errors::describe;
use crate::ops::temp_text;
use crate::outcome::{Outcome, refusal, success};
use crate::run::git;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::LazyLock;

pub const MODES: [&str; 3] = ["stage", "unstage", "discard"];

const NO_NEWLINE: &str = "\\ No newline at end of file";

static HUNK_HEADER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@(.*)$").expect("hunk header"));

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HunkHead {
    pub old_start: u64,
    pub old_count: u64,
    pub new_start: u64,
    pub new_count: u64,
    pub heading: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BodyLine {
    pub at: usize,
    pub sign: char,
    pub text: String,
    pub eol: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawHunk {
    pub header: String,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Split {
    pub header: Vec<String>,
    pub hunks: Vec<RawHunk>,
}

fn apply_args(mode: &str) -> &'static [&'static str] {
    match mode {
        "stage" => &["--cached"],
        "unstage" => &["--cached", "-R"],
        _ => &["-R"],
    }
}

fn fail_title(mode: &str) -> &'static str {
    match mode {
        "stage" => "Could not stage this",
        "unstage" => "Could not unstage this",
        "discard" => "Could not discard this",
        _ => "Could not do this",
    }
}

pub fn parse_hunk_header(line: &str) -> Option<HunkHead> {
    let m = HUNK_HEADER.captures(line)?;
    let num = |i: usize, default: u64| m.get(i).map(|v| v.as_str().parse().unwrap_or(0)).unwrap_or(default);
    Some(HunkHead {
        old_start: num(1, 0),
        old_count: num(2, 1),
        new_start: num(3, 0),
        new_count: num(4, 1),
        heading: m.get(5).map(|v| v.as_str().to_string()).unwrap_or_default(),
    })
}

pub fn hunk_body(lines: &[String]) -> Vec<BodyLine> {
    let mut body: Vec<BodyLine> = Vec::new();
    for (at, line) in lines.iter().enumerate().skip(1) {
        if line.starts_with('\\') {
            if let Some(last) = body.last_mut() {
                last.eol = false;
            }
            continue;
        }
        let sign = line.chars().next().unwrap_or(' ');
        if sign != ' ' && sign != '+' && sign != '-' {
            break;
        }
        let text = if line.is_empty() { String::new() } else { line[1..].to_string() };
        body.push(BodyLine { at, sign, text, eol: true });
    }
    body
}

pub fn split_hunks(text: &str) -> Split {
    let text = text.strip_suffix('\n').unwrap_or(text);
    let lines: Vec<&str> = text.split('\n').collect();
    let Some(first) = lines.iter().position(|l| l.starts_with("@@")) else { return Split::default() };
    let mut hunks: Vec<RawHunk> = Vec::new();
    for line in &lines[first..] {
        if line.starts_with("@@") {
            hunks.push(RawHunk { header: line.to_string(), lines: vec![line.to_string()] });
        } else if let Some(current) = hunks.last_mut() {
            current.lines.push(line.to_string());
        }
    }
    Split { header: lines[..first].iter().map(|l| l.to_string()).collect(), hunks }
}

pub fn change_indexes(lines: &[String]) -> Vec<usize> {
    hunk_body(lines).into_iter().filter(|l| l.sign != ' ').map(|l| l.at).collect()
}

pub fn hunk_complete(lines: &[String]) -> bool {
    let Some(head) = lines.first().and_then(|l| parse_hunk_header(l)) else { return false };
    let body = hunk_body(lines);
    let old = body.iter().filter(|l| l.sign != '+').count() as u64;
    let now = body.iter().filter(|l| l.sign != '-').count() as u64;
    old == head.old_count && now == head.new_count
}

fn sides(sign: char) -> &'static [&'static str] {
    match sign {
        ' ' => &["old", "new"],
        '-' => &["old"],
        _ => &["new"],
    }
}

pub fn ends_valid(out: &[BodyLine]) -> bool {
    for (i, line) in out.iter().enumerate() {
        if line.eol {
            continue;
        }
        let mine = sides(line.sign);
        if out[i + 1..].iter().any(|l| sides(l.sign).iter().any(|s| mine.contains(s))) {
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
    let minus: Vec<usize> = (s..=e).filter(|&k| body[k].sign == '-').collect();
    let plus: Vec<usize> = (s..=e).filter(|&k| body[k].sign == '+').collect();
    let at = minus.iter().position(|&k| k == index)?;
    plus.get(at).copied()
}

fn path_line(line: &str, prefix: &str, side: &str) -> String {
    let rest = line.get(4..).unwrap_or("");
    let quoted = rest.starts_with('"');
    let body = if quoted { &rest[1..] } else { rest };
    let tail = body.get(1..).unwrap_or("");
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
                (Some(p), true) => path_line(p, "---", "a"),
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
                (Some(m), true) => path_line(m, "+++", "b"),
                _ => l,
            })
            .collect();
    }
    lines
}

pub fn build_line_patch(header: &[String], lines: &[String], picked: &[usize], mode: &str) -> Result<String, String> {
    if !MODES.contains(&mode) {
        return Err("This action is not known.".into());
    }
    let head = lines.first().and_then(|l| parse_hunk_header(l));
    let Some(head) = head.filter(|_| hunk_complete(lines)) else {
        return Err("This part of the diff is not complete. Look at the diff again.".into());
    };
    let body = hunk_body(lines);
    let wanted: HashSet<usize> = picked.iter().copied().collect();
    let mut chosen: HashSet<usize> = (0..body.len()).filter(|&k| body[k].sign != ' ' && wanted.contains(&body[k].at)).collect();
    if chosen.is_empty() {
        return Err("Pick one or more added or removed lines first.".into());
    }
    let forward = mode == "stage";
    let mut extra: HashMap<usize, BodyLine> = HashMap::new();
    if forward
        && let Some(last) = body.iter().position(|l| l.sign == '-' && !l.eol)
        && !chosen.contains(&last)
        && (last + 1..body.len()).any(|k| body[k].sign == '+' && chosen.contains(&k))
    {
        chosen.insert(last);
        match partner_of(&body, last) {
            Some(p) if body[p].text == body[last].text => {
                chosen.insert(p);
            }
            _ => {
                extra.insert(last, BodyLine { at: 0, sign: '+', text: body[last].text.clone(), eol: true });
            }
        }
    }
    let mut out: Vec<BodyLine> = Vec::new();
    for (k, l) in body.iter().enumerate() {
        if l.sign == ' ' || chosen.contains(&k) {
            out.push(BodyLine { at: 0, sign: l.sign, text: l.text.clone(), eol: l.eol });
        } else if (forward && l.sign == '-') || (!forward && l.sign == '+') {
            out.push(BodyLine { at: 0, sign: ' ', text: l.text.clone(), eol: l.eol });
        }
        if let Some(more) = extra.get(&k) {
            out.push(more.clone());
        }
    }
    if !ends_valid(&out) {
        return Err("These lines touch the end of a file that has no newline at the end. Pick the lines after them too, or use the hunk buttons.".into());
    }
    let old_count = out.iter().filter(|l| l.sign != '+').count();
    let new_count = out.iter().filter(|l| l.sign != '-').count();
    let start = |s: u64, n: usize| if n > 0 && s == 0 { 1 } else { s };
    let top = format!(
        "@@ -{},{old_count} +{},{new_count} @@{}",
        start(head.old_start, old_count),
        start(head.new_start, new_count),
        head.heading
    );
    let mut text: Vec<String> = Vec::new();
    for l in &out {
        text.push(format!("{}{}", l.sign, l.text));
        if !l.eol {
            text.push(NO_NEWLINE.into());
        }
    }
    Ok(format!("{}\n{top}\n{}\n", file_header(header, old_count, new_count).join("\n"), text.join("\n")))
}

fn failed(mode: &str, text: &str) -> Outcome {
    let words = describe(text);
    if words.kind != "other" {
        return Outcome { ok: false, kind: words.kind, title: words.title, hint: words.hint, detail: words.detail, ..Outcome::default() };
    }
    refusal("apply", fail_title(mode), "The file may have changed. Look at the diff again, then try once more.").detail(&words.detail)
}

pub fn apply_patch(root: &Path, patch: &str, mode: &str) -> Outcome {
    if !MODES.contains(&mode) {
        return refusal("invalid", "This action is not known", "");
    }
    if patch.is_empty() || patch.contains('\0') {
        return refusal("invalid", fail_title(mode), "There is no text change to apply.");
    }
    let file = match temp_text("nixa-git-patch.", patch) {
        Ok(f) => f,
        Err(e) => return failed(mode, &e),
    };
    let path = file.path().to_string_lossy().into_owned();
    let mut args = vec!["apply"];
    args.extend_from_slice(apply_args(mode));
    args.extend(["--recount", "--whitespace=nowarn", path.as_str()]);
    let out = git(root, &args);
    if !out.ok() {
        return failed(mode, if out.stderr.is_empty() { &out.stdout } else { &out.stderr });
    }
    success("Done")
}

pub fn apply_lines(root: &Path, header: &[String], lines: &[String], picked: &[usize], mode: &str) -> Outcome {
    match build_line_patch(header, lines, picked, mode) {
        Ok(patch) => apply_patch(root, &patch, mode),
        Err(hint) => refusal("invalid", fail_title(mode), &hint),
    }
}

pub fn stage_lines(root: &Path, header: &[String], lines: &[String], picked: &[usize]) -> Outcome {
    apply_lines(root, header, lines, picked, "stage")
}

pub fn unstage_lines(root: &Path, header: &[String], lines: &[String], picked: &[usize]) -> Outcome {
    apply_lines(root, header, lines, picked, "unstage")
}

pub fn discard_lines(root: &Path, header: &[String], lines: &[String], picked: &[usize]) -> Outcome {
    apply_lines(root, header, lines, picked, "discard")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::git;
    use crate::testrepo;

    fn s(list: &[&str]) -> Vec<String> {
        list.iter().map(|l| l.to_string()).collect()
    }

    #[test]
    fn headers_and_bodies() {
        let h = parse_hunk_header("@@ -3 +3,2 @@ fn x").unwrap();
        assert_eq!((h.old_start, h.old_count, h.new_start, h.new_count), (3, 1, 3, 2));
        assert_eq!(h.heading, " fn x");
        let lines = s(&["@@ -1,2 +1,2 @@", " a", "-b", "\\ No newline at end of file", "+c"]);
        let body = hunk_body(&lines);
        assert_eq!(body.len(), 3);
        assert!(!body[1].eol);
        assert!(hunk_complete(&lines));
        assert_eq!(change_indexes(&lines), vec![2, 4]);
        let split = split_hunks("diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n@@ -9 +9 @@\n-c\n+d\n");
        assert_eq!(split.header.len(), 3);
        assert_eq!(split.hunks.len(), 2);
        assert_eq!(split.hunks[1].lines, s(&["@@ -9 +9 @@", "-c", "+d"]));
    }

    #[test]
    fn line_patch_keeps_unpicked_lines_as_context() {
        let header = s(&["diff --git a/a.txt b/a.txt", "index 1..2 100644", "--- a/a.txt", "+++ b/a.txt"]);
        let lines = s(&["@@ -1,3 +1,3 @@", "-one", "+ONE", " two", "-three", "+THREE"]);
        let patch = build_line_patch(&header, &lines, &[1, 2], "stage").unwrap();
        assert_eq!(patch, "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1,3 +1,3 @@\n-one\n+ONE\n two\n three\n");
        let back = build_line_patch(&header, &lines, &[5], "unstage").unwrap();
        assert!(back.ends_with("@@ -1,2 +1,3 @@\n ONE\n two\n+THREE\n"), "{back}");
        assert!(build_line_patch(&header, &lines, &[3], "stage").is_err());
        assert!(build_line_patch(&header, &lines, &[1], "x").is_err());
    }

    #[test]
    fn stage_and_discard_lines_on_a_repo() {
        let repo = testrepo::new();
        let root = repo.root();
        repo.write("a.txt", "ONE\ntwo\nTHREE\n");
        let diff = git(root, &["diff", "--no-color", "--", "a.txt"]).stdout;
        let split = split_hunks(&diff);
        let hunk = &split.hunks[0];
        let picked: Vec<usize> = hunk.lines.iter().enumerate().filter(|(_, l)| l.contains("ONE") || l.contains("one")).map(|(i, _)| i).collect();
        let out = stage_lines(root, &split.header, &hunk.lines, &picked);
        assert!(out.ok, "{out:?}");
        let staged = git(root, &["diff", "--cached", "--no-color"]).stdout;
        assert!(staged.contains("+ONE"));
        assert!(!staged.contains("THREE"));
        let left = git(root, &["diff", "--no-color", "--", "a.txt"]).stdout;
        let split = split_hunks(&left);
        let all = change_indexes(&split.hunks[0].lines);
        let gone = discard_lines(root, &split.header, &split.hunks[0].lines, &all);
        assert!(gone.ok, "{gone:?}");
        assert_eq!(repo.read("a.txt"), "ONE\ntwo\nthree\n");
        let whole = git(root, &["diff", "--cached", "--no-color"]).stdout;
        let un = apply_patch(root, &whole, "unstage");
        assert!(un.ok, "{un:?}");
        assert!(git(root, &["diff", "--cached"]).stdout.is_empty());
        assert_eq!(apply_patch(root, "", "stage").kind, "invalid");
        let bad = apply_patch(root, "diff --git a/zz b/zz\n--- a/zz\n+++ b/zz\n@@ -1 +1 @@\n-q\n+r\n", "stage");
        assert!(!bad.ok);
    }

    #[test]
    fn new_file_part_keeps_paths() {
        let header = s(&["diff --git a/n.txt b/n.txt", "new file mode 100644", "index 0..1", "--- /dev/null", "+++ b/n.txt"]);
        let out = file_header(&header, 1, 2);
        assert_eq!(out, s(&["diff --git a/n.txt b/n.txt", "--- a/n.txt", "+++ b/n.txt"]));
    }
}
