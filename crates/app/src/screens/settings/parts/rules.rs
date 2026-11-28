use super::answer_rules::{self, AnswerRulesView};
use super::rule_trees::{self, RuleTreesView};
use gpui::prelude::*;
use gpui::{App, Context, Entity, FontWeight, ScrollHandle, Window, div, px};
use nixa_ui::theme;
use serde_json::Value;

pub const PART: &str = "Rules";
pub const OLD_PARTS: [&str; 3] = ["Answer rules", "Rule trees", "Standing instructions"];
pub const INTRO: &str = "All your rules in one place. Standing rules always apply to every reply and job. Answer rules let Nixa answer an agent's repeated question by itself. Rule trees are step by step rules that Nixa follows when their \"when\" line fits.";

pub struct RulesView {
    pub standing: Entity<AnswerRulesView>,
    pub trees: Entity<RuleTreesView>,
    pub scroll: ScrollHandle,
}

pub fn is_part(part: &str) -> bool {
    part == PART || OLD_PARTS.contains(&part)
}

pub fn new(addr: String, folder: Option<String>, cx: &mut App) -> Entity<RulesView> {
    let standing = answer_rules::new(addr.clone(), folder, cx);
    let trees = rule_trees::new(addr, cx);
    cx.new(|cx| {
        cx.observe(&standing, |_, _, cx| cx.notify()).detach();
        cx.observe(&trees, |_, _, cx| cx.notify()).detach();
        RulesView {
            standing,
            trees,
            scroll: ScrollHandle::new(),
        }
    })
}

pub fn feed(view: &Entity<RulesView>, msg: &Value, cx: &mut App) {
    let (standing, trees) = {
        let view = view.read(cx);
        (view.standing.clone(), view.trees.clone())
    };
    answer_rules::feed(&standing, msg, cx);
    rule_trees::feed(&trees, msg, cx);
}

impl Render for RulesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        div()
            .id("rules-page")
            .size_full()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .flex()
            .flex_col()
            .gap(px(14.))
            .pb(px(24.))
            .child(
                div()
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.bright)
                    .child(PART),
            )
            .child(div().w_full().text_size(px(11.5)).text_color(t.dim).child(INTRO))
            .child(self.standing.clone())
            .child(self.trees.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_part_names_open_rules() {
        assert!(is_part("Rules"));
        assert!(is_part("Answer rules"));
        assert!(is_part("Rule trees"));
        assert!(is_part("Standing instructions"));
        assert!(!is_part("Memory"));
    }
}
