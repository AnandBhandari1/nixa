use crate::ctx::Ctx;
use anyhow::Result;
pub use nixa_protocol::caller::Caller;
use nixa_store::tasks::{Actor, AuthorKind};

pub fn who(ctx: &Ctx) -> Result<Caller> {
    nixa_protocol::caller::who(&ctx.socket)
}

pub fn actor_of(caller: &Caller) -> Actor {
    match caller {
        Caller::Agent(id) => Actor::in_terminal(AuthorKind::Agent, id, id),
        Caller::User => {
            let name = std::env::var("USER").ok().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "owner".into());
            Actor::new(AuthorKind::User, &name)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actor_identity_is_preserved() {
        assert_eq!(actor_of(&Caller::Agent("m3".into())).terminal, "m3");
        assert_eq!(actor_of(&Caller::User).kind, AuthorKind::User);
    }
}
