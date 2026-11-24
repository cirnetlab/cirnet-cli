use std::{
    borrow::Cow,
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::{
    localization::{self, Diagnostic, message, text},
    process::{capture, isolate_git, run},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Repo {
    Cli,
    Contracts,
    Simulator,
    Hardware,
    Remote,
    Interface,
    Embedded,
}

impl Repo {
    pub const ALL: [Self; 7] = [
        Self::Cli,
        Self::Contracts,
        Self::Simulator,
        Self::Hardware,
        Self::Remote,
        Self::Interface,
        Self::Embedded,
    ];
    pub const KEYS: [&str; 7] = [
        "cli",
        "contracts",
        "simulator",
        "hardware",
        "remote",
        "interface",
        "embedded",
    ];

    pub fn key(self) -> &'static str {
        Self::KEYS[self as usize]
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Cli => "cirnet-cli",
            Self::Contracts => "cirnet-contracts",
            Self::Simulator => "cirnet-simulator",
            Self::Hardware => "cirnet-hardware",
            Self::Remote => "cirnet-remote",
            Self::Interface => "cirnet-interface",
            Self::Embedded => "cirnet-embedded-hardware",
        }
    }

    pub fn parse(key: &str) -> Self {
        Self::ALL[Self::KEYS
            .iter()
            .position(|candidate| *candidate == key)
            .expect("validated repository key")]
    }

    fn dependencies(self) -> &'static [Self] {
        match self {
            Self::Hardware => &[Self::Contracts, Self::Simulator],
            Self::Remote => &[Self::Contracts, Self::Simulator, Self::Hardware],
            Self::Interface => &[
                Self::Contracts,
                Self::Simulator,
                Self::Hardware,
                Self::Remote,
            ],
            Self::Embedded => &[Self::Contracts],
            _ => &[],
        }
    }

    pub fn ordered(selected: &[Self]) -> Vec<Self> {
        Self::ALL
            .into_iter()
            .filter(|repo| {
                selected
                    .iter()
                    .any(|selected| selected == repo || selected.dependencies().contains(repo))
            })
            .collect()
    }

    pub fn url(self) -> String {
        format!("https://github.com/cirnetlab/{}.git", self.name())
    }
}

pub struct Workspace {
    pub root: PathBuf,
}

pub(crate) fn user_path(path: &Path) -> Cow<'_, str> {
    let text = path.to_string_lossy();
    if let Some(remainder) = text.strip_prefix(r"\\?\") {
        if remainder
            .get(..4)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(r"UNC\"))
        {
            return Cow::Owned(format!(r"\\{}", &remainder[4..]));
        }
        if matches!(remainder.as_bytes(), [letter, b':', b'\\', ..] if letter.is_ascii_alphabetic())
        {
            return match text {
                Cow::Borrowed(text) => Cow::Borrowed(&text[4..]),
                Cow::Owned(text) => Cow::Owned(text[4..].to_owned()),
            };
        }
    }
    text
}

pub fn canonical(path: &Path) -> Result<PathBuf> {
    let absolute = path.canonicalize().with_context(|| {
        Diagnostic::new(|vi| {
            format!(
                "{} {}",
                text(localization::PATH_RESOLUTION_FAILED, vi),
                path.display()
            )
        })
    })?;
    let Some(text) = absolute.to_str() else {
        return Ok(absolute);
    };
    if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
        return Ok(PathBuf::from(format!(r"\\{unc}")));
    }
    Ok(text
        .strip_prefix(r"\\?\")
        .map(PathBuf::from)
        .unwrap_or(absolute))
}

impl Workspace {
    pub fn discover(root: Option<&Path>) -> Result<Self> {
        if let Some(root) = root {
            return Ok(Self {
                root: canonical(root)?,
            });
        }
        if let Some(root) = env::var_os("CNL_WORKSPACE").filter(|value| !value.is_empty()) {
            return Self::discover(Some(Path::new(&root)));
        }
        for origin in [env::current_dir(), env::current_exe()]
            .into_iter()
            .flatten()
        {
            if let Ok(path) = canonical(&origin)
                && let Some(root) = path.ancestors().find(|ancestor| {
                    ancestor.join(Repo::Remote.name()).is_dir()
                        || ancestor.join(Repo::Cli.name()).join(".git").exists()
                })
            {
                return Ok(Self {
                    root: root.to_owned(),
                });
            }
        }
        bail!("{}", message(localization::WORKSPACE_MISSING))
    }

    pub fn path(&self, repo: Repo) -> PathBuf {
        self.root.join(repo.name())
    }

    pub fn require(&self, repo: Repo) -> Result<PathBuf> {
        let path = self.path(repo);
        let metadata = fs::symlink_metadata(&path).with_context(|| {
            Diagnostic::new(|vi| {
                format!(
                    "{} {}",
                    text(localization::CHECKOUT_MISSING, vi),
                    path.display()
                )
            })
        })?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            bail!(Diagnostic::new(|vi| format!(
                "{} {}",
                text(localization::CHECKOUT_REQUIRED, vi),
                path.display()
            )));
        }
        let root = capture(git(&path).args(["rev-parse", "--show-toplevel"]))?;
        if canonical(Path::new(&root))? != canonical(&path)? {
            bail!(Diagnostic::new(|vi| format!(
                "{} {}",
                path.display(),
                text(localization::CHECKOUT_NOT_INDEPENDENT, vi)
            )));
        }
        Ok(path)
    }

    pub fn inventory(&self, detailed: bool) -> Vec<Value> {
        Repo::ALL.into_iter().map(|repo| {
            let path = self.path(repo);
            let result = (|| -> Result<Value> {
                let path = self.require(repo)?;
                let mut result = json!({"repo": repo.name(), "key": repo.key(), "path": path, "state": "checkout"});
                if detailed {
                    let branch = capture(git(&path).args(["branch", "--show-current"]))?;
                    let revision = capture(git(&path).args(["rev-parse", "HEAD"]))?;
                    let changes = capture(git(&path).args(["status", "--porcelain=v1"]))?;
                    let remotes = capture(git(&path).args(["remote", "-v"]))?;
                    let remotes = remotes.lines().map(redact_remote).collect::<Vec<_>>();
                    let dependencies = repo.dependencies().iter().map(|dependency| {
                        json!({"repo": dependency.name(), "present": self.require(*dependency).is_ok() })
                    }).collect::<Vec<_>>();
                    result["branch"] = if branch.is_empty() {
                        localization::report_message(localization::REPO_DETACHED_HEAD)
                    } else {
                        json!(branch)
                    };
                    result["revision"] = json!(revision);
                    result["changes"] = json!(changes.lines().collect::<Vec<_>>());
                    result["remotes"] = json!(remotes);
                    result["dependencies"] = json!(dependencies);
                }
                Ok(result)
            })();
            result.unwrap_or_else(|error| json!({
                "repo": repo.name(), "key": repo.key(), "path": path,
                "state": if path.exists() {"invalid"} else {"missing"},
                "detail": localization::report_diagnostic(&error)
            }))
        }).collect()
    }

    pub fn mutate(&self, action: &str, selected: &[Repo], dry: bool) -> Result<Vec<Value>> {
        // Inspect every requested operation before the first mutation.
        for &repo in selected {
            let path = self.path(repo);
            if action == "clone" {
                if fs::symlink_metadata(&path).is_ok() {
                    bail!(
                        "{} {}",
                        message(localization::CLONE_DESTINATION_EXISTS),
                        path.display()
                    );
                }
                continue;
            }
            let path = self.require(repo)?;
            if !capture(git(&path).args(["status", "--porcelain=v1"]))?.is_empty() {
                bail!("{} {}", repo.name(), message(localization::CHECKOUT_DIRTY));
            }
            let branch = capture(git(&path).args(["branch", "--show-current"]))?;
            let mut key = format!("branch.{branch}.remote");
            if branch.is_empty() || capture(git(&path).args(["config", "--get", &key]))? != "origin"
            {
                bail!(
                    "{} {}",
                    repo.name(),
                    message(localization::CHECKOUT_BRANCH_ORIGIN)
                );
            }
            self.verify_origin(repo, &path)?;
            key = format!("branch.{branch}.merge");
            if !capture(git(&path).args(["config", "--get", &key]))?.starts_with("refs/heads/") {
                bail!(
                    "{} {}",
                    repo.name(),
                    message(localization::CHECKOUT_UPSTREAM_MISSING)
                );
            }
        }
        let mut report = Vec::new();
        for &repo in selected {
            if !dry {
                if action == "clone" {
                    run(git(&self.root)
                        .arg("clone")
                        .arg("--")
                        .arg(repo.url())
                        .arg(repo.name()))?;
                } else {
                    run(git(&self.path(repo)).args([
                        "-c",
                        "merge.autostash=false",
                        "pull",
                        "--ff-only",
                        "--no-rebase",
                    ]))?;
                }
            }
            report.push(json!({"repo": repo.name(), "operation": action, "dryRun": dry}));
        }
        Ok(report)
    }

    pub fn missing_checkouts(&self) -> Result<Vec<Repo>> {
        let mut missing = Vec::new();
        for repo in Repo::ALL {
            match fs::symlink_metadata(self.path(repo)) {
                Ok(_) => self.verify_origin(repo, &self.require(repo)?)?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => missing.push(repo),
                Err(error) => return Err(error.into()),
            }
        }
        Ok(missing)
    }

    fn verify_origin(&self, repo: Repo, path: &Path) -> Result<()> {
        let url = capture(git(path).args(["remote", "get-url", "origin"]))?;
        if url.trim_end_matches(".git") != repo.url().trim_end_matches(".git")
            && url.trim_end_matches(".git") != format!("git@github.com:cirnetlab/{}", repo.name())
        {
            bail!(
                "{} {}",
                repo.name(),
                message(localization::CHECKOUT_ORIGIN_MISMATCH)
            );
        }
        Ok(())
    }
}

fn git(path: &Path) -> Command {
    let mut command = Command::new("git");
    command.args(["--no-optional-locks", "-C"]).arg(path);
    isolate_git(&mut command);
    command
}

fn redact_remote(line: &str) -> String {
    let mut parts = line.split_whitespace();
    let (Some(name), Some(url), Some(operation)) = (parts.next(), parts.next(), parts.next())
    else {
        return "unrecognized remote".into();
    };
    let url = if let Some((protocol, rest)) = url.split_once("://") {
        let rest = rest.split(['?', '#']).next().unwrap_or("");
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        let host = authority.rsplit('@').next().unwrap_or_default();
        format!("{protocol}://{host}/{path}")
    } else {
        url.to_owned()
    };
    format!("{name} {url} {operation}")
}

#[cfg(test)]
mod path_tests {
    use super::*;

    #[test]
    fn user_paths_keep_windows_drives_and_unc_shares_readable() {
        for (internal, visible) in [
            (r"\\?\E:\repo có dấu", r"E:\repo có dấu"),
            (r"\\?\UNC\server\share\repo", r"\\server\share\repo"),
            (r"\\?\unc\server\share\repo", r"\\server\share\repo"),
            (r"\\server\share\repo", r"\\server\share\repo"),
            (r"\\?\Volume{example}\repo", r"\\?\Volume{example}\repo"),
            ("/tmp/workspace", "/tmp/workspace"),
        ] {
            let path = PathBuf::from(internal);
            assert_eq!(user_path(&path), visible);
            assert_eq!(path.as_os_str(), internal);
        }
    }
}
