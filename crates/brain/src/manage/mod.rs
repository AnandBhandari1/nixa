pub mod diagnostics;
pub mod go_live;
pub mod repeats;
pub mod schedules;
pub mod settings;
pub mod usage;

#[cfg(test)]
mod tests;

use crate::app::App;
use crate::tools::{Args, ToolResult};
use crate::turns::ToolSpec;
use std::sync::Arc;

pub const NAMES: [&str; 6] = [settings::NAME, schedules::NAME, repeats::NAME, go_live::NAME, usage::NAME, diagnostics::NAME];

pub fn specs() -> Vec<ToolSpec> {
    vec![settings::spec_of(), schedules::spec_of(), repeats::spec_of(), go_live::spec_of(), usage::spec_of(), diagnostics::spec_of()]
}

pub async fn call(app: &Arc<App>, name: &str, args: &Args) -> Option<ToolResult> {
    let result = match name {
        settings::NAME => settings::call(app, args).await,
        schedules::NAME => schedules::call(app, args).await,
        repeats::NAME => repeats::call(app, args).await,
        go_live::NAME => go_live::call(app, args),
        usage::NAME => usage::call(app, args),
        diagnostics::NAME => diagnostics::call(app),
        _ => return None,
    };
    Some(result)
}
