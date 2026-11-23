use std::{
    collections::HashSet,
    io,
    process::{Command, Stdio},
};

use serde::{Deserialize, Serialize};

use super::{
    messages as msg,
    registry::{now, token, valid_slug},
};

const ORG: &str = "cirnetlab";
pub(super) const CACHE: &str = "github-catalog.json";
const STALE_AFTER: u64 = 24 * 60 * 60;
const CATALOG_VERSION: u16 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Remote {
    pub id: u64,
    #[serde(rename = "full_name")]
    pub slug: String,
    pub default_branch: String,
    pub private: bool,
    pub archived: bool,
}

impl Remote {
    pub fn name(&self) -> &str {
        self.slug
            .split_once('/')
            .map_or(&self.slug, |(_, name)| name)
    }
    pub fn url(&self) -> String {
        format!("https://github.com/{}.git", self.slug)
    }
    fn valid(&self) -> bool {
        self.id != 0
            && valid_slug(&self.slug)
            && self
                .slug
                .split_once('/')
                .is_some_and(|(owner, _)| owner.eq_ignore_ascii_case(ORG))
            && !self.default_branch.chars().any(char::is_control)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Catalog {
    version: u16,
    pub refreshed: u64,
    pub repositories: Vec<Remote>,
}

impl Catalog {
    pub fn stale(&self) -> bool {
        now().saturating_sub(self.refreshed) >= STALE_AFTER
    }

    pub fn valid(&self) -> bool {
        let mut ids = HashSet::new();
        let mut slugs = HashSet::new();
        self.version == CATALOG_VERSION
            && self.repositories.iter().all(|entry| {
                entry.valid()
                    && ids.insert(entry.id)
                    && slugs.insert(entry.slug.to_ascii_lowercase())
            })
    }

    pub fn get(&self, key: &str) -> Option<&Remote> {
        if !valid_slug(key) && !token(key) {
            return None;
        }
        self.repositories.iter().find(|entry| {
            entry.slug.eq_ignore_ascii_case(key) || entry.name().eq_ignore_ascii_case(key)
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Failure {
    Missing,
    Auth,
    Forbidden,
    Rate,
    Network,
    Data,
}

impl Failure {
    pub fn code(self) -> &'static str {
        match self {
            Self::Missing => "cli-unavailable",
            Self::Auth => "authentication-required",
            Self::Forbidden => "permission-denied",
            Self::Rate => "rate-limited",
            Self::Network => "unavailable",
            Self::Data => "invalid-response",
        }
    }

    pub fn message(self) -> crate::localization::Localized {
        match self {
            Self::Missing => msg::GH_MISSING,
            Self::Auth => msg::GH_AUTH,
            Self::Forbidden => msg::GH_FORBIDDEN,
            Self::Rate => msg::GH_RATE,
            Self::Network => msg::GH_UNAVAILABLE,
            Self::Data => msg::GH_BAD_DATA,
        }
    }
}

struct Reply {
    ok: bool,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

pub(super) fn fetch() -> Result<Catalog, Failure> {
    fetch_with(|args| {
        let output = Command::new("gh")
            .args(args)
            .env("GH_PROMPT_DISABLED", "1")
            .env("GH_HOST", "github.com")
            .env_remove("GH_DEBUG")
            .stdin(Stdio::null())
            .output()
            .map_err(|error| {
                if error.kind() == io::ErrorKind::NotFound {
                    Failure::Missing
                } else {
                    Failure::Network
                }
            })?;
        Ok(Reply {
            ok: output.status.success(),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    })
}

fn fetch_with(mut run: impl FnMut(&[&str]) -> Result<Reply, Failure>) -> Result<Catalog, Failure> {
    let auth = run(&["auth", "status", "--hostname", "github.com", "--active"])?;
    if !auth.ok {
        return Err(classify(&auth.stderr).unwrap_or(Failure::Auth));
    }
    let result = run(&[
        "api",
        "--hostname",
        "github.com",
        "--method",
        "GET",
        "--paginate",
        "--slurp",
        "orgs/cirnetlab/repos?type=all&per_page=100",
    ])?;
    if !result.ok {
        return Err(classify(&result.stderr).unwrap_or(Failure::Network));
    }
    let pages: Vec<Vec<Remote>> =
        serde_json::from_slice(&result.stdout).map_err(|_| Failure::Data)?;
    let mut records = std::collections::BTreeMap::new();
    for entry in pages.into_iter().flatten() {
        if !entry.valid() {
            return Err(Failure::Data);
        }
        records.insert(entry.id, entry);
    }
    let mut repositories: Vec<_> = records.into_values().collect();
    repositories.sort_by_cached_key(|entry| entry.slug.to_ascii_lowercase());
    let catalog = Catalog {
        version: CATALOG_VERSION,
        refreshed: now(),
        repositories,
    };
    if catalog.valid() {
        Ok(catalog)
    } else {
        Err(Failure::Data)
    }
}

fn classify(stderr: &[u8]) -> Option<Failure> {
    let text = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    if text.contains("rate limit") || text.contains("http 429") {
        Some(Failure::Rate)
    } else if text.contains("http 401") || text.contains("bad credentials") {
        Some(Failure::Auth)
    } else if text.contains("http 403") || text.contains("http 404") {
        Some(Failure::Forbidden)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_every_page_and_keeps_accessible_private_repositories() {
        let pages: Vec<Vec<_>> = [1..=100, 101..=157]
            .into_iter()
            .map(|page| {
                page.map(|id| {
                    json!({
                        "id":id,"full_name":format!("cirnetlab/repo-{id}"),"private":id == 155,
                        "default_branch":"main","archived":false,"token":"must never persist"
                    })
                })
                .collect()
            })
            .collect();
        let response = serde_json::to_vec(&pages).unwrap();
        let catalog = fetch_with(|args| {
            assert!(args.contains(&"github.com"));
            if args[0] == "api" {
                assert!(args.contains(&"--paginate"));
                assert!(args.contains(&"--slurp"));
            }
            Ok(Reply {
                ok: true,
                stdout: response.clone(),
                stderr: vec![],
            })
        })
        .unwrap();
        assert_eq!(catalog.repositories.len(), 157);
        assert!(catalog.get("repo-155").unwrap().private);
        assert!(
            !serde_json::to_string(&catalog)
                .unwrap()
                .contains("must never persist")
        );
    }

    #[test]
    fn distinguishes_authentication_permission_and_rate_limits() {
        for (problem, failure) in [
            ("bad credentials (HTTP 401)", Failure::Auth),
            ("forbidden (HTTP 403)", Failure::Forbidden),
            ("primary rate limit exceeded (HTTP 403)", Failure::Rate),
            ("connection timed out", Failure::Network),
        ] {
            assert_eq!(
                fetch_with(|args| Ok(Reply {
                    ok: args[0] == "auth",
                    stdout: vec![],
                    stderr: problem.as_bytes().to_vec()
                }))
                .unwrap_err(),
                failure
            );
        }
        assert_eq!(
            fetch_with(|_| Err(Failure::Missing)).unwrap_err(),
            Failure::Missing
        );
        assert_eq!(
            fetch_with(|_| Ok(Reply {
                ok: false,
                stdout: vec![],
                stderr: vec![]
            }))
            .unwrap_err(),
            Failure::Auth
        );
    }
}
