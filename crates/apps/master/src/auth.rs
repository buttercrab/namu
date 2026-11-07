//! Who may call the API: bearer tokens, each with a role. `client`s publish tasks and workflows,
//! start runs and read them; `worker`s join the mesh and fetch what they run. Publishing a task
//! is publishing code every worker runs, so nobody without a token reaches it.
//!
//! The orchestrator keeps only tokens' SHA-256 hashes, from a file of lines `<role> sha256:<hex>`
//! (`namu token` makes a token and its line). Listing several per role lets a token be rotated:
//! add the new one, move clients over, remove the old. The file is read again when it changes.

use std::path::Path;
use std::sync::RwLock;

use anyhow::Context;
use axum::extract::{Request, State};
use axum::http::header::AUTHORIZATION;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use sha2::{Digest, Sha256};

use crate::error::ApiError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Client,
    Worker,
}

impl Role {
    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "client" => Some(Self::Client),
            "worker" => Some(Self::Worker),
            _ => None,
        }
    }
}

/// The tokens the API takes, as hashes.
#[derive(Debug, Default)]
pub struct Tokens(RwLock<Vec<Entry>>);

impl Tokens {
    pub fn from_file(path: &Path) -> anyhow::Result<Self> {
        Ok(Self(RwLock::new(read(path)?)))
    }

    /// Takes the tokens in `path` instead, unless it holds none that parse.
    pub fn reload_file(&self, path: &Path) -> anyhow::Result<()> {
        let tokens = read(path)?;
        *self.0.write().unwrap() = tokens;
        Ok(())
    }

    #[cfg(test)]
    fn parse(text: &str) -> anyhow::Result<Self> {
        Ok(Self(RwLock::new(parse(text)?)))
    }

    /// The role `token` has, if it is one of these. Every hash is compared, in full, so how long
    /// it takes says nothing about which one matched.
    pub fn role(&self, token: &str) -> Option<Role> {
        let digest: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        let mut found = None;
        for (role, hash) in self.0.read().unwrap().iter() {
            let same = hash.iter().zip(&digest).fold(0u8, |d, (a, b)| d | (a ^ b)) == 0;
            if same {
                found = Some(*role);
            }
        }
        found
    }
}

type Entry = (Role, [u8; 32]);

fn read(path: &Path) -> anyhow::Result<Vec<Entry>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading tokens from {}", path.display()))?;
    parse(&text).with_context(|| format!("tokens in {}", path.display()))
}

fn parse(text: &str) -> anyhow::Result<Vec<Entry>> {
    let mut tokens = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let entry = || -> Option<Entry> {
            let (role, hash) = line.split_once(char::is_whitespace)?;
            let hex = hash.trim().strip_prefix("sha256:")?;
            let bytes = (0..64)
                .step_by(2)
                .map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok())
                .collect::<Option<Vec<u8>>>()?;
            Some((Role::parse(role)?, bytes.try_into().ok()?))
        };
        let entry = entry().with_context(|| {
            format!(
                "line {}: expected `client|worker sha256:<64 hex digits>`",
                n + 1
            )
        })?;
        tokens.push(entry);
    }
    anyhow::ensure!(!tokens.is_empty(), "no tokens");
    Ok(tokens)
}

/// What a route requires: one of `roles`, or nothing if the API runs without tokens.
#[derive(Clone)]
pub struct Require {
    pub tokens: Option<std::sync::Arc<Tokens>>,
    pub roles: &'static [Role],
}

/// Lets a request through if it carries a token with one of the route's roles.
pub async fn require(State(require): State<Require>, request: Request, next: Next) -> Response {
    let Some(tokens) = &require.tokens else {
        return next.run(request).await;
    };
    let token = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    match token.and_then(|token| tokens.role(token)) {
        Some(role) if require.roles.contains(&role) => next.run(request).await,
        Some(_) => ApiError::Forbidden("this token's role may not do that".into()).into_response(),
        None => ApiError::Unauthorized(
            "a valid token is required: `Authorization: Bearer <token>`".into(),
        )
        .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(role: &str, token: &str) -> String {
        format!("{role} sha256:{:x}\n", Sha256::digest(token.as_bytes()))
    }

    #[test]
    fn tokens_have_their_roles() {
        let text = format!(
            "# rotated on 2026-11-07\n{}{}\n{}",
            line("client", "c1"),
            line("client", "c2"),
            line("worker", "w1")
        );
        let tokens = Tokens::parse(&text).unwrap();
        assert_eq!(tokens.role("c1"), Some(Role::Client));
        assert_eq!(tokens.role("c2"), Some(Role::Client));
        assert_eq!(tokens.role("w1"), Some(Role::Worker));
        assert_eq!(tokens.role("w2"), None);
        assert_eq!(tokens.role(""), None);
    }

    #[test]
    fn a_bad_line_is_refused() {
        for bad in ["admin sha256:00", "client 1234", "client sha256:xyz", ""] {
            assert!(Tokens::parse(bad).is_err(), "{bad:?}");
        }
    }
}
