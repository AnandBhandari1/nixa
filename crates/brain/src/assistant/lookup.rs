use super::answer;
use super::route::{self, Route};
use super::sources;
use crate::app::App;
use crate::schedules::{jobs as sched, when};
use std::sync::Arc;

pub fn stamp(app: &App) -> String {
    when::date_time(&sched::zone_of(app), sched::now())
}

pub fn names(app: &App) -> Vec<String> {
    let (owner, aliases) = crate::privacy::owner_of(app);
    std::iter::once(owner).chain(aliases).filter(|n| !n.trim().is_empty()).collect()
}

async fn local(app: &Arc<App>, question: &str, memory_first: bool) -> String {
    if (memory_first || !sources::knowledge_ready(app))
        && let Some(text) = answer::memory_text(&sources::memory(app, question, 2), &stamp(app))
    {
        return text;
    }
    let hits = sources::knowledge(app, question, 2).await;
    answer::knowledge_text(&hits).unwrap_or_else(|| answer::NO_LOCAL.to_string())
}

pub async fn live(app: &Arc<App>, query: &str) -> String {
    let web = sources::web(app, query, 2).await;
    answer::live_text(&sources::web_name(app), query, &web, &stamp(app))
}

async fn plan_of(app: &Arc<App>, question: &str, names: &[String]) -> route::Routing {
    let rule = route::route(question, names);
    if rule.route == Route::Connector || !crate::settings_api::nixa_on(&app.dir) {
        return rule;
    }
    let set = crate::decide::settings::load(app);
    if !set.uses("chat") {
        return rule;
    }
    let questions = vec![crate::decide::choice(route::ROUTE_PICK, route::ROUTE_TEXT, &route::ROUTE_CHOICES)];
    let Ok(answers) = crate::decide::ask(app, &set.model, vec![crate::decide::text(question)], questions).await else { return rule };
    match answers.get(route::ROUTE_PICK) {
        Some(crate::decide::Answer::Choice { choice, confidence, .. }) if *confidence >= set.level => match choice.as_str().and_then(route::route_of_word) {
            Some(picked) => route::with_model(rule, picked),
            None => rule,
        },
        _ => rule,
    }
}

pub async fn answer(app: &Arc<App>, question: &str, public: Option<&str>) -> String {
    let names = names(app);
    let plan = plan_of(app, question, &names).await;
    match plan.route {
        Route::Web => live(app, question).await,
        Route::Knowledge | Route::Memory => local(app, question, plan.personal).await,
        Route::Mix => {
            let saved = local(app, question, true).await;
            match public.map(str::trim).filter(|p| !p.is_empty()) {
                Some(p) if !route::route(p, &names).personal => answer::join(&saved, &live(app, p).await),
                _ => answer::join(&saved, answer::SPLIT_PUBLIC),
            }
        }
        Route::Connector => {
            let saved = local(app, question, true).await;
            let found = sources::personal(app, question, route::is_mail(question)).await;
            answer::join(&saved, &answer::personal_text(found, &stamp(app)))
        }
    }
}

pub fn ready(app: &App, question: &str) -> bool {
    match route::route(question, &names(app)).route {
        Route::Web => sources::web_ready(app),
        Route::Connector => sources::personal_ready(app),
        Route::Knowledge => sources::knowledge_ready(app),
        Route::Memory | Route::Mix => false,
    }
}
