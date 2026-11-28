use super::limits::{find, limit_line, parse_given, parse_until};
use crate::schedules::zone::{self, Zone};

const HOUR: u64 = 3_600_000;
const MIN: u64 = 60_000;

fn utc() -> Zone {
    zone::utc()
}

fn at(y: i64, m: i64, d: i64, h: u64, min: u64) -> u64 {
    zone::days_from_civil(y, m, d) as u64 * 86_400_000 + h * HOUR + min * MIN
}

fn now() -> u64 {
    at(2026, 11, 26, 10, 0)
}

fn screen(lines: &[&str]) -> String {
    let mut all: Vec<&str> = lines.to_vec();
    all.extend(["", "────────────────────────", "❯ ", "────────────────────────", "  ? for shortcuts"]);
    all.join("\n")
}

#[test]
fn claude_limit_lines_are_found_with_their_reset_time() {
    let s = screen(&["⏺ Working on the parser.", "  ⎿  You've hit your limit · resets 3pm (Etc/UTC)"]);
    let hit = find(&s, now(), &utc()).unwrap();
    assert!(hit.line.contains("You've hit your limit"));
    assert_eq!(hit.until, Some(at(2026, 11, 26, 15, 0)));
    assert!(hit.before.contains("Working on the parser"));
    let weekly = find(&screen(&["Weekly limit reached ∙ resets Mon 9am"]), now(), &utc()).unwrap();
    assert_eq!(weekly.until, Some(at(2026, 11, 30, 9, 0)));
    let hours = find(&screen(&["5-hour limit reached ∙ resets 5pm"]), now(), &utc()).unwrap();
    assert_eq!(hours.until, Some(at(2026, 11, 26, 17, 0)));
    let old = find(&screen(&["  ⎿  Claude AI usage limit reached|1796220000"]), now(), &utc()).unwrap();
    assert_eq!(old.until, Some(1_796_220_000_000));
    let dated = find(&screen(&["Claude usage limit reached. Your limit will reset at Dec 2, 10am."]), now(), &utc()).unwrap();
    assert_eq!(dated.until, Some(at(2026, 12, 2, 10, 0)));
}

#[test]
fn codex_limit_lines_are_found() {
    let line = "■ You've hit your usage limit. Upgrade to Pro (https://openai.com/chatgpt/pricing), visit https://chatgpt.com/codex/settings/usage to purchase more credits or try again at 5:12 PM.";
    let hit = find(&screen(&["• Ran the tests", line]), now(), &utc()).unwrap();
    assert_eq!(hit.until, Some(at(2026, 11, 26, 17, 12)));
    let span = "🖐 You've hit your usage limit. Upgrade to Pro or try again in 1 day 2 hours 5 minutes.";
    let hit = find(&screen(&[span]), now(), &utc()).unwrap();
    assert_eq!(hit.until, Some(now() + 26 * HOUR + 5 * MIN));
}

#[test]
fn gemini_antigravity_opencode_and_crush_lines_are_found() {
    let gemini = "✕ [API Error: You have exhausted your capacity on this model. Your quota will reset after 2h13m.]";
    assert_eq!(find(&screen(&[gemini]), now(), &utc()).unwrap().until, Some(now() + 2 * HOUR + 13 * MIN));
    let quota = "✕ [API Error: Quota exceeded for quota metric 'Gemini 2.5 Pro Requests' and limit 'Requests per day'.]";
    assert_eq!(find(&screen(&[quota]), now(), &utc()).unwrap().until, None);
    assert!(find(&screen(&["Usage limit reached for all Pro models."]), now(), &utc()).is_some());
    assert!(find(&screen(&["Error: You exceeded your current quota, please check your plan and billing details."]), now(), &utc()).is_some());
    assert!(find(&screen(&["AI_APICallError: insufficient_quota"]), now(), &utc()).is_some());
    assert!(find(&screen(&["ERROR  rate limit exceeded, try again in 45 minutes"]), now(), &utc()).is_some_and(|h| h.until == Some(now() + 45 * MIN)));
    assert!(find(&screen(&["Error: Your credit balance is too low to access the Anthropic API."]), now(), &utc()).is_some());
    assert!(find(&screen(&["You are out of credits. Add more credits to go on."]), now(), &utc()).is_some());
}

#[test]
fn talk_and_code_about_limits_is_not_a_hit() {
    let quiet = [
        "⏺ I added detection for the usage limit message, like You've hit your limit.",
        "• Next I will handle the weekly limit reached case.",
        "const HIT: &str = \"usage limit reached\";",
        "  ⎿  src/leave/limits.rs:12: \"You've hit your usage limit\"",
        "    assert!(limit_line(\"5-hour limit reached ∙ resets 5pm\"));",
        "- weekly limit reached: move the task",
        "1. usage limit reached, then hand over",
        "Approaching usage limit · resets at 5pm",
        "You have used 90% of your weekly limit.",
        "Context limit reached: run /compact to go on.",
        "The usage limit handling is done and the tests pass.",
        "grep -rn usage limit reached src",
        "  You've hit your limit is the text, so it resets the task when it shows.",
        "  weekly limit reached, then it will try again at 5pm.",
        "› handle the usage limit reset and try again",
        "› You've hit your usage limit. Try again in 3h.",
        "  └ Error: rate limit exceeded, try again in 45 minutes",
        "└ ■ You've hit your usage limit. Try again in 3h.",
        "  ⎿  API rate limit exceeded for user ID 1234. Try again later.",
        "  ⎿  Error: 429 You exceeded your current quota, please check your plan and billing details.",
        "  ⎿  insufficient_quota: try again after the reset",
        "Rate limit exceeded, try again in 45 minutes.",
    ];
    for line in quiet {
        assert!(!limit_line(line), "false hit: {line}");
        assert!(find(&screen(&[line]), now(), &utc()).is_none(), "false hit: {line}");
    }
}

#[test]
fn an_old_hit_far_above_the_bottom_is_ignored() {
    let mut lines = vec!["■ You've hit your usage limit. Try again in 3h."];
    lines.extend(std::iter::repeat_n("• more work happened here", 14));
    assert!(find(&screen(&lines), now(), &utc()).is_none());
}

#[test]
fn reset_times_parse_in_many_forms() {
    let z = utc();
    assert_eq!(parse_until("resets 9am", now(), &z), Some(at(2026, 11, 27, 9, 0)));
    assert_eq!(parse_until("resets at 17:30", now(), &z), Some(at(2026, 11, 26, 17, 30)));
    assert_eq!(parse_until("try again in 3h", now(), &z), Some(now() + 3 * HOUR));
    assert_eq!(parse_until("try again in 2 days", now(), &z), Some(now() + 48 * HOUR));
    assert_eq!(parse_until("resets Thursday at 11am", now(), &z), Some(at(2026, 11, 26, 11, 0)));
    assert_eq!(parse_until("resets Thursday at 9am", now(), &z), Some(at(2026, 12, 3, 9, 0)));
    assert_eq!(parse_until("resets Oct 9, 10am", now(), &z), Some(at(2027, 10, 9, 10, 0)));
    assert_eq!(parse_until("limit reached", now(), &z), None);
    assert_eq!(parse_until("resets 13pm", now(), &z), None);
    let kolkata = parse_until("resets 3pm (Asia/Kolkata)", now(), &z);
    if zone::named("Asia/Kolkata").is_some() {
        assert_eq!(kolkata, Some(at(2026, 11, 26, 9, 30) + 24 * HOUR));
    }
}

#[test]
fn given_times_from_the_cli_parse() {
    let z = utc();
    assert_eq!(parse_given("5pm", now(), &z), Some(at(2026, 11, 26, 17, 0)));
    assert_eq!(parse_given("3h", now(), &z), Some(now() + 3 * HOUR));
    assert_eq!(parse_given("45m", now(), &z), Some(now() + 45 * MIN));
    assert_eq!(parse_given("monday 9am", now(), &z), Some(at(2026, 11, 30, 9, 0)));
    assert_eq!(parse_given("", now(), &z), None);
    assert_eq!(parse_given("soon", now(), &z), None);
}
