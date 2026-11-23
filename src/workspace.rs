use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::{
    localization::message,
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
            Self::Contracts => "cirnet-contracts-next",
            Self::Simulator => "cirnet-simulator-next",
            Self::Hardware => "cirnet-hardware-next",
            Self::Remote => "cirnet-remote-next",
            Self::Interface => "cirnet-interface-next",
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

pub fn canonical(path: &Path) -> Result<PathBuf> {
    let absolute = path.canonicalize().with_context(|| {
        format!(
            "{} {}",
            message(["Cannot resolve", "Không thể phân giải"]),
            path.display()
        )
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
        bail!(
            "{}",
            message([
                "Could not find the CIRNETLAB workspace. Set --workspace <directory> or CNL_WORKSPACE.",
                "Không tìm thấy workspace CIRNETLAB. Đặt --workspace <thư-mục> hoặc CNL_WORKSPACE."
            ])
        )
    }

    pub fn path(&self, repo: Repo) -> PathBuf {
        self.root.join(repo.name())
    }

    pub fn require(&self, repo: Repo) -> Result<PathBuf> {
        let path = self.path(repo);
        let metadata = fs::symlink_metadata(&path).with_context(|| {
            format!(
                "{} {}",
                message(["Checkout missing:", "Thiếu checkout tại"]),
                path.display()
            )
        })?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            bail!(
                "{} {}",
                message(["Expected a real checkout:", "Cần checkout Git thực tại"]),
                path.display()
            );
        }
        let root = capture(git(&path).args(["rev-parse", "--show-toplevel"]))?;
        if canonical(Path::new(&root))? != canonical(&path)? {
            bail!(
                "{} {}",
                path.display(),
                message([
                    "is not an independent Git checkout",
                    "không phải checkout Git độc lập"
                ])
            );
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
                    result["branch"] = json!(if branch.is_empty() { "detached" } else { &branch });
                    result["revision"] = json!(revision);
                    result["changes"] = json!(changes.lines().collect::<Vec<_>>());
                    result["remotes"] = json!(remotes);
                    result["dependencies"] = json!(dependencies);
                }
                Ok(result)
            })();
            result.unwrap_or_else(|error| json!({
                "repo": repo.name(), "key": repo.key(), "path": path,
                "state": if path.exists() {"invalid"} else {"missing"}, "detail": error.to_string()
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
                        message([
                            "Clone destination already exists:",
                            "Đường dẫn clone đã tồn tại"
                        ]),
                        path.display()
                    );
                }
                continue;
            }
            let path = self.require(repo)?;
            if !capture(git(&path).args(["status", "--porcelain=v1"]))?.is_empty() {
                bail!(
                    "{} {}",
                    repo.name(),
                    message([
                        "has local changes. Commit or stash them before updating.",
                        "có thay đổi cục bộ. Hãy commit hoặc stash trước khi cập nhật."
                    ])
                );
            }
            let branch = capture(git(&path).args(["branch", "--show-current"]))?;
            let mut key = format!("branch.{branch}.remote");
            if branch.is_empty() || capture(git(&path).args(["config", "--get", &key]))? != "origin"
            {
                bail!(
                    "{} {}",
                    repo.name(),
                    message([
                        "must be on a branch tracking origin",
                        "phải ở nhánh theo dõi origin"
                    ])
                );
            }
            self.verify_origin(repo, &path)?;
            key = format!("branch.{branch}.merge");
            if !capture(git(&path).args(["config", "--get", &key]))?.starts_with("refs/heads/") {
                bail!(
                    "{} {}",
                    repo.name(),
                    message(["has no branch upstream", "chưa có nhánh upstream"])
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
                message([
                    "origin is not the expected CIRNETLAB repository",
                    "có origin khác repo CIRNETLAB đã định"
                ])
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
