use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{self, Read},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Result, anyhow, ensure};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};

use crate::{
    environment::{State, write_json},
    workspace::{Workspace, canonical},
};

use super::messages as msg;

const SCHEMA: u16 = 1;
const FILE_LIMIT: u64 = 32 * 1024 * 1024;
const REGISTRY_FILE: &str = "registry.json";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Record {
    pub id: String,
    pub name: String,
    pub slug: Option<String>,
    pub github_id: Option<u64>,
    pub alias: Option<String>,
    pub path: Option<PathBuf>,
    pub remote: String,
    pub remote_url: Option<String>,
    pub preferred_branch: Option<String>,
    pub shell: Option<String>,
    pub groups: BTreeSet<String>,
    pub last_verified: Option<u64>,
    pub last_sync: Option<u64>,
    pub last_branch: Option<String>,
    pub last_state: Option<String>,
}

impl Record {
    pub fn new(name: String) -> Self {
        Self {
            id: String::new(),
            name,
            slug: None,
            github_id: None,
            alias: None,
            path: None,
            remote: "origin".into(),
            remote_url: None,
            preferred_branch: None,
            shell: None,
            groups: BTreeSet::new(),
            last_verified: None,
            last_sync: None,
            last_branch: None,
            last_state: None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Registry {
    version: u16,
    workspace: PathBuf,
    next_id: u64,
    pub records: Vec<Record>,
    pub groups: BTreeSet<String>,
    pub active: Option<String>,
}

impl Registry {
    fn new(workspace: PathBuf) -> Self {
        Self {
            version: SCHEMA,
            workspace,
            next_id: 1,
            records: vec![],
            groups: BTreeSet::new(),
            active: None,
        }
    }

    fn validate(&self, workspace: &Path) -> Result<()> {
        ensure!(
            self.version == SCHEMA && same_path(&self.workspace, workspace),
            "{}",
            msg::get(msg::REGISTRY_SCHEMA)
        );
        let mut ids = BTreeSet::new();
        let mut aliases = BTreeSet::new();
        let mut paths = BTreeSet::new();
        for entry in &self.records {
            let id = entry
                .id
                .strip_prefix("r:")
                .and_then(|s| s.parse::<u64>().ok());
            let valid = id.is_some_and(|n| n > 0 && n < self.next_id)
                && ids.insert(entry.id.as_str())
                && valid_text(&entry.name)
                && token(&entry.remote)
                && entry.slug.as_deref().is_none_or(valid_slug)
                && entry
                    .alias
                    .as_ref()
                    .is_none_or(|alias| token(alias) && aliases.insert(alias.to_ascii_lowercase()))
                && entry
                    .path
                    .as_ref()
                    .is_none_or(|path| path.is_absolute() && paths.insert(path_key(path)))
                && entry.remote_url.as_deref().is_none_or(|url| {
                    url.starts_with("https://github.com/")
                        && url
                            .strip_prefix("https://github.com/")
                            .and_then(|value| value.strip_suffix(".git"))
                            .is_some_and(valid_slug)
                })
                && entry.preferred_branch.as_deref().is_none_or(valid_text)
                && entry
                    .shell
                    .as_deref()
                    .is_none_or(|shell| crate::cli::native::SHELLS.contains(&shell))
                && entry.groups.is_subset(&self.groups);
            ensure!(valid, "{}", msg::get(msg::REGISTRY_INVALID));
        }
        ensure!(
            self.groups.iter().all(|name| token(name))
                && self.active.as_deref().is_none_or(|id| ids.contains(id)),
            "{}",
            msg::get(msg::REGISTRY_INVALID)
        );
        Ok(())
    }

    pub fn insert(&mut self, mut entry: Record) -> Result<String> {
        entry.id = format!("r:{}", self.next_id);
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| anyhow!("{}", msg::get(msg::REGISTRY_INVALID)))?;
        let id = entry.id.clone();
        self.records.push(entry);
        Ok(id)
    }

    pub fn index(&self, input: &str, root: &Path) -> Result<Option<usize>> {
        if let Some(index) = self.records.iter().position(|record| {
            record.id == input
                || record
                    .alias
                    .as_deref()
                    .is_some_and(|alias| alias.eq_ignore_ascii_case(input))
        }) {
            return Ok(Some(index));
        }
        let path = root.join(input);
        let actual = path.is_dir().then(|| canonical(&path).ok()).flatten();
        if let Some(index) = self.records.iter().position(|entry| {
            entry.path.as_ref().is_some_and(|registered| {
                same_path(registered, &path)
                    || actual
                        .as_ref()
                        .is_some_and(|actual| same_path(registered, actual))
            })
        }) {
            return Ok(Some(index));
        }
        let mut found = self.records.iter().enumerate().filter(|(_, entry)| {
            entry.slug.as_deref().is_some_and(|slug| {
                slug.eq_ignore_ascii_case(input)
                    || slug
                        .rsplit_once('/')
                        .is_some_and(|(_, name)| name.eq_ignore_ascii_case(input))
            }) || entry.name.eq_ignore_ascii_case(input)
        });
        let result = found.next().map(|(index, _)| index);
        ensure!(
            found.next().is_none(),
            "{} {input}",
            msg::get(msg::AMBIGUOUS)
        );
        Ok(result)
    }
}

pub(super) struct Store {
    pub directory: PathBuf,
    workspace: PathBuf,
}

impl Store {
    pub fn resolve(workspace: &Workspace, external: Option<&Path>) -> Result<Self> {
        let input = if cfg!(windows) {
            workspace.root.to_string_lossy().to_lowercase()
        } else {
            workspace.root.to_string_lossy().into_owned()
        };
        let key = format!("{:x}", Sha256::digest(input));
        let scope = format!("repositories-{}", &key[..24]);
        let platform = if cfg!(windows) || external.is_some() {
            None
        } else {
            std::env::var_os("XDG_STATE_HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .or_else(|| {
                    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state"))
                })
                .map(|path| path.join("cirnetlab"))
        };
        let state = State::resolve(workspace, external.or(platform.as_deref()), &scope)?;
        Ok(Self {
            directory: state.dir,
            workspace: workspace.root.clone(),
        })
    }

    pub fn load(&self) -> Result<Registry> {
        let registry = self
            .read(REGISTRY_FILE)?
            .unwrap_or_else(|| Registry::new(self.workspace.clone()));
        registry.validate(&self.workspace)?;
        Ok(registry)
    }

    pub fn edit<R>(&self, change: impl FnOnce(&mut Registry) -> Result<R>) -> Result<R> {
        let _lock = self.lock()?;
        let mut registry = self.load()?;
        let result = change(&mut registry)?;
        registry.validate(&self.workspace)?;
        self.write(REGISTRY_FILE, &registry)?;
        Ok(result)
    }

    pub fn lock(&self) -> Result<File> {
        fs::create_dir_all(&self.directory)?;
        ensure!(
            same_path(&canonical(&self.directory)?, &self.directory),
            "{}",
            msg::get(msg::REGISTRY_LOCATION)
        );
        let path = self.directory.join("registry.lock");
        ordinary_file(&path)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        file.lock()?;
        Ok(file)
    }

    pub fn read<T: DeserializeOwned>(&self, name: &str) -> Result<Option<T>> {
        let file = self.directory.join(name);
        if !ordinary_file(&file)? {
            return Ok(None);
        }
        let mut bytes = Vec::new();
        File::open(&file)?
            .take(FILE_LIMIT + 1)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= FILE_LIMIT,
            "{}",
            msg::get(msg::REGISTRY_INVALID)
        );
        Ok(Some(serde_json::from_slice(&bytes).map_err(|_| {
            anyhow!("{}", msg::get(msg::REGISTRY_INVALID))
        })?))
    }

    pub fn write(&self, name: &str, value: &impl Serialize) -> Result<()> {
        let path = self.directory.join(name);
        ordinary_file(&path)?;
        write_json(&path, value)
    }
}

fn ordinary_file(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            ensure!(
                meta.is_file()
                    && !meta.file_type().is_symlink()
                    && same_path(&canonical(path)?, path),
                "{}",
                msg::get(msg::REGISTRY_LOCATION)
            );
            Ok(true)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(super) fn token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && !matches!(value, "." | "..")
        && !value.starts_with('-')
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
}

pub(super) fn valid_slug(value: &str) -> bool {
    value
        .split_once('/')
        .is_some_and(|(owner, repo)| token(owner) && token(repo))
}

pub(super) fn valid_text(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}

fn path_key(path: &Path) -> String {
    let normalized: PathBuf = path.components().collect();
    let value = normalized.to_string_lossy();
    if cfg!(windows) {
        value.to_lowercase()
    } else {
        value.into_owned()
    }
}

pub(super) fn same_path(a: &Path, b: &Path) -> bool {
    path_key(a) == path_key(b)
}
