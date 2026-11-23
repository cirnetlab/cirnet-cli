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
    localization::message,
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
            bail!(
                "{}",
                message([
                    "HTTP, Hardware RPC and Vite ports must be distinct and in 1024..=65535",
                    "Cổng HTTP, Hardware RPC và Vite phải khác nhau và nằm trong 1024..=65535"
                ])
            );
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
            bail!(
                "{}",
                message([
                    "Use a profile name with letters, digits, hyphens or underscores",
                    "Tên hồ sơ chỉ dùng chữ cái, chữ số, dấu gạch ngang hoặc gạch dưới"
                ])
            );
        }
        let base = match selected {
            Some(path) => {
                if !path.is_absolute() {
                    bail!(
                        "{}",
                        message([
                            "--state-dir requires an absolute external path",
                            "--state-dir yêu cầu đường dẫn tuyệt đối nằm ngoài repo"
                        ])
                    );
                }
                path.to_owned()
            }
            None => {
                let root = env::var_os("LOCALAPPDATA")
                    .map(PathBuf::from)
                    .context(message([
                        "Set --state-dir when LOCALAPPDATA is unavailable",
                        "Đặt --state-dir khi không có LOCALAPPDATA",
                    ]))?;
                let key = format!(
                    "{:x}",
                    Sha256::digest(workspace.root.to_string_lossy().to_lowercase())
                );
                root.join("CIRNETLAB").join(&key[..16])
            }
        };
        let mut present = base.as_path();
        while !present.exists() {
            present = present.parent().context(message([
                "State path has no existing ancestor",
                "Đường dẫn trạng thái không có thư mục cha đang tồn tại",
            ]))?;
        }
        let tail = base.strip_prefix(present)?;
        if tail
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
        {
            bail!(
                "{}",
                message([
                    "State path must use ordinary directory names",
                    "Đường dẫn trạng thái phải dùng tên thư mục thông thường"
                ])
            );
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
            bail!(
                "{}",
                message([
                    "State and artifacts must be outside repository and Git directories",
                    "Trạng thái và artifact phải nằm ngoài repo và thư mục Git"
                ])
            );
        }
        let profile_name = profile.to_owned();
        let dir = resolved.join(profile);
        if let Ok(info) = fs::symlink_metadata(&dir)
            && (!info.is_dir() || canonical(&dir)? != dir)
        {
            bail!(
                "{}",
                message([
                    "The profile directory must be an ordinary directory",
                    "Thư mục hồ sơ phải là thư mục thông thường"
                ])
            );
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
        file.try_lock().context(message([
            "Another cnl operation is using this profile",
            "Một thao tác cnl khác đang dùng hồ sơ này",
        ]))?;
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
                Err(error) => json!({"tool":name,"ok":false,"detail":error.to_string()}),
            }
        })
        .collect::<Vec<_>>();
    let msvc = vswhere().and_then(|exe| capture(Command::new(exe).args(VS_ARGS)));
    checks.push(match msvc {
        Ok(path) if !path.is_empty() => json!({"tool":"msvc","ok":true,"path":path}),
        _ => json!({"tool":"msvc","ok":false,"detail":message(["Install VS 2022 with the x64 C++ tools and Windows SDK", "Cài VS 2022 với công cụ C++ x64 và Windows SDK"])}),
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
        .context(message([
            "Visual Studio installation directory unavailable",
            "Không tìm thấy thư mục cài đặt Visual Studio",
        ]))?
        .join("Microsoft Visual Studio/Installer/vswhere.exe");
    if !path.is_file() {
        bail!(
            "{} {}",
            message([
                "Visual Studio Installer not found at",
                "Không tìm thấy Visual Studio Installer tại"
            ]),
            path.display()
        );
    }
    Ok(path)
}

pub fn install_browser(receipt: &Path, cache: &Path) -> Result<()> {
    let interface = receipt
        .parent()
        .context(message([
            "Expected build receipt path",
            "Cần đường dẫn receipt của bản dựng",
        ]))?
        .join("src/cirnet-interface-next");
    let cli = interface.join("node_modules/@playwright/test/cli.js");
    if !cli.is_file() {
        bail!(
            "{}",
            message([
                "The selected candidate does not include a prepared Playwright checkout",
                "Ứng viên đã chọn thiếu checkout đã chuẩn bị cho Playwright"
            ])
        );
    }
    run(Command::new("node")
        .arg(cli)
        .args(["install", "chromium"])
        .current_dir(interface)
        .env("PLAYWRIGHT_BROWSERS_PATH", cache))
}
