use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{
    localization::{self, message},
    process::{capture, run},
    workspace::{Repo, Workspace, canonical},
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Profile {
    pub port: u16,
    pub hardware_port: u16,
    pub dev_port: u16,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            port: 8000,
            hardware_port: 50051,
            dev_port: 5173,
        }
    }
}

impl Profile {
    pub fn validate(&self) -> Result<()> {
        let ports = [self.port, self.hardware_port, self.dev_port];
        if ports.iter().any(|port| *port < 1024)
            || self.port == self.hardware_port
            || self.port == self.dev_port
            || self.hardware_port == self.dev_port
        {
            bail!("{}", message(localization::PROFILE_PORTS));
        }
        Ok(())
    }
}

pub struct State {
    pub dir: PathBuf,
    pub name: String,
    pub profile: Profile,
}

impl State {
    pub fn resolve(workspace: &Workspace, selected: Option<&Path>, profile: &str) -> Result<Self> {
        if profile.is_empty()
            || !profile
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            bail!("{}", message(localization::PROFILE_NAME));
        }
        let base = match selected {
            Some(path) => {
                if !path.is_absolute() {
                    bail!("{}", message(localization::STATE_ABSOLUTE));
                }
                path.to_owned()
            }
            None => {
                let root = env::var_os("LOCALAPPDATA")
                    .map(PathBuf::from)
                    .context(message(localization::STATE_LOCALAPPDATA))?;
                let key = format!(
                    "{:x}",
                    Sha256::digest(workspace.root.to_string_lossy().to_lowercase())
                );
                root.join("CIRNETLAB").join(&key[..16])
            }
        };
        let mut present = base.as_path();
        while !present.exists() {
            present = present
                .parent()
                .context(message(localization::STATE_ANCESTOR))?;
        }
        let tail = base.strip_prefix(present)?;
        if tail
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
        {
            bail!("{}", message(localization::STATE_DIRECTORY_NAMES));
        }
        let resolved = canonical(present)?.join(tail);
        if resolved.ancestors().any(|part| {
            part.file_name()
                .is_some_and(|name| name.eq_ignore_ascii_case(".git"))
                || part.join(".git").exists()
        }) || Repo::ALL
            .iter()
            .any(|repo| contains_ignoring_case(&workspace.path(*repo), &resolved))
            || resolved.exists() && !resolved.is_dir()
        {
            bail!("{}", message(localization::STATE_OUTSIDE_GIT));
        }
        let profile_name = profile.to_owned();
        let dir = resolved.join(profile);
        if let Ok(info) = fs::symlink_metadata(&dir)
            && (!info.is_dir() || canonical(&dir)? != dir)
        {
            bail!("{}", message(localization::PROFILE_DIRECTORY));
        }
        let profile_path = dir.join("profile.json");
        let profile = if profile_path.exists() {
            serde_json::from_slice(&fs::read(profile_path)?)?
        } else {
            Profile::default()
        };
        let state = Self {
            dir,
            name: profile_name,
            profile,
        };
        state.profile.validate()?;
        Ok(state)
    }

    pub fn prepare(&self) -> Result<()> {
        Ok(fs::create_dir_all(&self.dir)?)
    }

    pub fn lock(&self) -> Result<fs::File> {
        self.prepare()?;
        let file = fs::OpenOptions::new()
            .write(true)
            .read(true)
            .create(true)
            .truncate(false)
            .open(self.dir.join("operation.lock"))?;
        file.try_lock()
            .context(message(localization::PROFILE_IN_USE))?;
        Ok(file)
    }

    pub fn save_profile(&self) -> Result<()> {
        let _lock = self.lock()?;
        write_json(&self.dir.join("profile.json"), &self.profile)
    }
}

fn contains_ignoring_case(parent: &Path, child: &Path) -> bool {
    let parent = parent.components().collect::<Vec<_>>();
    child
        .components()
        .zip(&parent)
        .all(|(a, b)| a.as_os_str().eq_ignore_ascii_case(b.as_os_str()))
        && child.components().count() >= parent.len()
}

pub fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    let moment = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let temporary = path.with_extension(format!("json.{}-{moment}.new", std::process::id()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let written = file.write_all(&bytes).and_then(|()| file.sync_all());
    drop(file);
    let result = written.and_then(|()| fs::rename(&temporary, path));
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    Ok(result?)
}

pub fn doctor(workspace: &Workspace) -> Value {
    let tools = [
        ("git", "git", "--version", ""),
        ("rustup", "rustup", "--version", ""),
        ("rustc", "rustc", "--version", "rustc 1.99.1 "),
        ("cargo", "cargo", "--version", "cargo 1.99.1 "),
        ("cmake", "cmake", "--version", "cmake version "),
        ("node", "node", "--version", "v26."),
        ("npm", "npm.cmd", "--version", ""),
        ("powershell", "pwsh.exe", "--version", "PowerShell "),
    ];
    let mut checks = tools
        .into_iter()
        .map(|(name, executable, flag, expected)| {
            let mut command = Command::new(executable);
            command.arg(flag);
            if matches!(name, "rustc" | "cargo") {
                command.current_dir(workspace.path(Repo::Contracts));
            }
            let result = capture(&mut command);
            match result {
                Ok(version) => {
                    let identified = version.starts_with(expected);
                    let supported = name != "powershell"
                        || version
                            .split_whitespace()
                            .nth(1)
                            .and_then(|version| version.split_once('.'))
                            .is_some_and(|(major, minor)| {
                                major == "7"
                                    && minor
                                        .split('.')
                                        .next()
                                        .and_then(|n| n.parse::<u32>().ok())
                                        .is_some_and(|n| n >= 4)
                            });
                    json!({"tool":name,"ok":identified && supported,"version":version})
                }
                Err(error) => {
                    json!({"tool":name,"ok":false,"detail":localization::report_diagnostic(&error)})
                }
            }
        })
        .collect::<Vec<_>>();
    let msvc = vswhere().and_then(|exe| capture(Command::new(exe).args(VS_ARGS)));
    checks.push(match msvc {
        Ok(path) if !path.is_empty() => json!({"tool":"msvc","ok":true,"path":path}),
        _ => json!({"tool":"msvc","ok":false,"detail":localization::report_message(localization::DOCTOR_MSVC)}),
    });
    let platform = cfg!(all(windows, target_arch = "x86_64"));
    json!({"supportedPlatform":platform,"os":env::consts::OS,"architecture":env::consts::ARCH,
        "ok": platform && checks.iter().all(|item| item["ok"] == true), "tools":checks})
}

pub const VS_ARGS: [&str; 9] = [
    "-latest",
    "-products",
    "*",
    "-version",
    "[17.0,18.0)",
    "-requires",
    "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
    "-property",
    "installationPath",
];

pub fn vswhere() -> Result<PathBuf> {
    let path = env::var_os("ProgramFiles(x86)")
        .map(PathBuf::from)
        .context(message(localization::VS_DIRECTORY_MISSING))?
        .join("Microsoft Visual Studio/Installer/vswhere.exe");
    if !path.is_file() {
        bail!(
            "{} {}",
            message(localization::VS_INSTALLER_MISSING),
            path.display()
        );
    }
    Ok(path)
}

pub fn install_browser(receipt: &Path, cache: &Path) -> Result<()> {
    let interface = receipt
        .parent()
        .context(message(localization::BUILD_RECEIPT_REQUIRED))?
        .join("src/cirnet-interface-next");
    let cli = interface.join("node_modules/@playwright/test/cli.js");
    if !cli.is_file() {
        bail!("{}", message(localization::PLAYWRIGHT_MISSING));
    }
    run(Command::new("node")
        .arg(cli)
        .args(["install", "chromium"])
        .current_dir(interface)
        .env("PLAYWRIGHT_BROWSERS_PATH", cache))
}
