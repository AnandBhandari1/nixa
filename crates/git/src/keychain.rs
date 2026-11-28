use crate::accounts::Env;
use crate::accounts::run::{self, Opts, Output};

pub fn account_key(host: &str, username: &str) -> String {
    format!("acct:{host}:{username}")
}

pub fn valid_token(token: &str) -> bool {
    !token.is_empty()
        && token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_.~+/=:-".contains(c))
}

fn valid_user(user: &str) -> bool {
    !user.is_empty()
        && user
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}

pub fn valid_host(host: &str) -> bool {
    let (name, port) = match host.split_once(':') {
        Some((name, port)) => (name, Some(port)),
        None => (host, None),
    };
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        && port.is_none_or(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

fn valid_service(service: &str) -> bool {
    !service.is_empty() && !service.contains(['"', '\\', '\r', '\n'])
}

pub fn check_parts(env: &Env, host: &str, username: &str) -> Result<(), String> {
    if !valid_host(host) {
        return Err(format!("\"{host}\" is not a valid host name."));
    }
    if !valid_user(username) {
        return Err(format!("\"{username}\" is not a valid user name."));
    }
    if !valid_service(&env.service) {
        return Err("The Keychain service name is not valid.".into());
    }
    Ok(())
}

fn security(env: &Env, args: &[&str], stdin: Option<String>) -> Output {
    run::run(
        &env.security,
        &run::strings(args),
        &Opts {
            stdin,
            timeout: Some(std::time::Duration::from_secs(20)),
            ..Opts::default()
        },
    )
}

pub fn save_token(env: &Env, host: &str, username: &str, token: &str) -> Result<(), String> {
    check_parts(env, host, username)?;
    let clean = token.trim();
    if !valid_token(clean) {
        return Err("This does not look like a token. Copy it again and paste it.".into());
    }
    let line = format!(
        "add-generic-password -U -s \"{}\" -a \"{}\" -w \"{}\"\n",
        env.service,
        account_key(host, username),
        clean
    );
    let out = security(env, &["-i"], Some(line));
    if !out.ok() {
        return Err(format!(
            "The Keychain did not save the token. {}",
            run::last_line(&out.stderr)
        )
        .trim()
        .to_string());
    }
    if !has_token(env, host, username)? {
        return Err("The token did not reach the Keychain.".into());
    }
    Ok(())
}

pub fn read_token(env: &Env, host: &str, username: &str) -> Result<Option<String>, String> {
    check_parts(env, host, username)?;
    let key = account_key(host, username);
    let out = security(
        env,
        &["find-generic-password", "-s", &env.service, "-a", &key, "-w"],
        None,
    );
    let token = out.stdout.trim().to_string();
    Ok((out.ok() && !token.is_empty()).then_some(token))
}

pub fn has_token(env: &Env, host: &str, username: &str) -> Result<bool, String> {
    check_parts(env, host, username)?;
    let key = account_key(host, username);
    let out = security(
        env,
        &["find-generic-password", "-s", &env.service, "-a", &key],
        None,
    );
    Ok(out.ok())
}

pub fn delete_token(env: &Env, host: &str, username: &str) -> Result<bool, String> {
    check_parts(env, host, username)?;
    let key = account_key(host, username);
    let out = security(
        env,
        &["delete-generic-password", "-s", &env.service, "-a", &key],
        None,
    );
    match out.code {
        Some(0) => Ok(true),
        Some(44) => Ok(false),
        _ => Err(format!(
            "The Keychain did not delete the token. {}",
            run::last_line(&out.stderr)
        )
        .trim()
        .to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::testkit;

    #[test]
    fn tokens_and_parts_are_checked() {
        assert!(valid_token("tok_abcDEF123"));
        assert!(!valid_token("has space"));
        assert!(!valid_token("quote\"x"));
        assert!(valid_host("github.com"));
        assert!(valid_host("git.example.com:8443"));
        assert!(!valid_host("bad host"));
        assert!(!valid_host("x:"));
        let kit = testkit::kit();
        assert!(check_parts(&kit.env, "github.com", "ana-b").is_ok());
        assert!(check_parts(&kit.env, "github.com", "a b").is_err());
    }

    #[test]
    fn save_read_delete_with_the_fake_keychain() {
        let kit = testkit::kit();
        let env = &kit.env;
        assert_eq!(read_token(env, "github.com", "ana").unwrap(), None);
        save_token(env, "github.com", "ana", "  tok_one  ").unwrap();
        assert_eq!(read_token(env, "github.com", "ana").unwrap().as_deref(), Some("tok_one"));
        assert!(has_token(env, "github.com", "ana").unwrap());
        save_token(env, "github.com", "ana", "tok_two").unwrap();
        assert_eq!(read_token(env, "github.com", "ana").unwrap().as_deref(), Some("tok_two"));
        assert!(!kit.security_log().contains("tok_two"));
        assert!(delete_token(env, "github.com", "ana").unwrap());
        assert!(!delete_token(env, "github.com", "ana").unwrap());
        assert!(save_token(env, "github.com", "ana", "bad token").is_err());
    }

    #[test]
    fn a_gh_login_is_never_copied_in() {
        let kit = testkit::kit();
        kit.gh_token("ana", "ghtok_fromgh");
        assert_eq!(read_token(&kit.env, "github.com", "ana").unwrap(), None);
        assert!(!has_token(&kit.env, "github.com", "ana").unwrap());
    }
}
