use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{
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
            bail!("HTTP, Hardware RPC and Vite ports must be distinct and in 1024..=65535");
        }
        Ok(())
    }
}

pub struct State {
    pub dir: PathBuf,
    pub profile: Profile,
}

impl State {
    pub fn resolve(workspace: &Workspace, selected: Option<&Path>, profile: &str) -> Result<Self> {
        if profile.is_empty()
            || !profile
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            bail!("Use a profile name with letters, digits, hyphens or underscores");
        }
        let base = match selected {
            Some(path) => {
                if !path.is_absolute() {
                    bail!("--state-dir requires an absolute external path");
                }
                path.to_owned()
            }
            None => {
                let root = env::var_os("LOCALAPPDATA")
                    .map(PathBuf::from)
                    .context("Set --state-dir when LOCALAPPDATA is unavailable")?;
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
                .context("State path has no existing ancestor")?;
        }
        let tail = base.strip_prefix(present)?;
        if tail
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
        {
            bail!("State path must use ordinary directory names");
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
            bail!("State and artifacts must be outside repository and Git directories");
        }
        let dir = resolved.join(profile);
        if let Ok(info) = fs::symlink_metadata(&dir)
            && (!info.is_dir() || canonical(&dir)? != dir)
        {
            bail!("The profile directory must be an ordinary directory");
        }
        let profile_path = dir.join("profile.json");
        let profile = if profile_path.exists() {
            serde_json::from_slice(&fs::read(profile_path)?)?
        } else {
            Profile::default()
        };
        let state = Self { dir, profile };
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
            .context("Another cnl operation is using this profile")?;
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
    let temporary = path.with_extension("json.new");
    fs::write(&temporary, serde_json::to_vec_pretty(value)?)?;
    fs::rename(temporary, path)?;
    Ok(())
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
                Err(error) => json!({"tool":name,"ok":false,"detail":error.to_string()}),
            }
        })
        .collect::<Vec<_>>();
    let msvc = vswhere().and_then(|exe| capture(Command::new(exe).args(VS_ARGS)));
    checks.push(match msvc {
        Ok(path) if !path.is_empty() => json!({"tool":"msvc","ok":true,"path":path}),
        _ => json!({"tool":"msvc","ok":false,"detail":"Install VS 2022 with the x64 C++ tools and Windows SDK"}),
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
        .context("Visual Studio installation directory unavailable")?
        .join("Microsoft Visual Studio/Installer/vswhere.exe");
    if !path.is_file() {
        bail!("Visual Studio Installer not found at {}", path.display());
    }
    Ok(path)
}

pub fn install_browser(receipt: &Path, cache: &Path) -> Result<()> {
    let interface = receipt
        .parent()
        .context("Expected build receipt path")?
        .join("src/cirnet-interface-next");
    let cli = interface.join("node_modules/@playwright/test/cli.js");
    if !cli.is_file() {
        bail!("The selected candidate does not include a prepared Playwright checkout");
    }
    run(Command::new("node")
        .arg(cli)
        .args(["install", "chromium"])
        .current_dir(interface)
        .env("PLAYWRIGHT_BROWSERS_PATH", cache))
}
