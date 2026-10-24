//! Where the ticker keeps its files.
//!
//! Like kroak-time, everything lives in a per-user folder rather than the
//! current directory: `%APPDATA%\kroak-time-ticker\config\` on Windows (next
//! to kroak-time's `%APPDATA%\kroak-time\config\`). The data folder is the
//! folder holding the config file, so `--config some/dir/x.toml` keeps that
//! run's state in `some/dir/` instead. Relative paths in the config
//! (`catalog_path`, `contract_path`) resolve against it too.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub const CONFIG_FILE: &str = "kroak-time-ticker.toml";

static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

/// The per-user config folder an app called `app` gets — the same place
/// kroak-time's settings library (confy) puts kroak-time's.
pub fn app_config_dir(app: &str) -> Option<PathBuf> {
    if cfg!(windows) {
        Some(PathBuf::from(std::env::var_os("APPDATA")?).join(app).join("config"))
    } else if cfg!(target_os = "macos") {
        Some(PathBuf::from(std::env::var_os("HOME")?).join(format!("Library/Application Support/rs.{app}")))
    } else {
        match std::env::var_os("XDG_CONFIG_HOME") {
            Some(x) => Some(PathBuf::from(x).join(app)),
            None => Some(PathBuf::from(std::env::var_os("HOME")?).join(".config").join(app)),
        }
    }
}

/// The config file to use: `--config` if given, else `kroak-time-ticker.toml`
/// in the ticker's own config folder. Sets the data folder to its folder.
pub fn init(config_arg: Option<&Path>) -> PathBuf {
    let config = match config_arg {
        Some(p) => std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf()),
        None => app_config_dir("kroak-time-ticker")
            .unwrap_or_else(|| PathBuf::from(".kroak-time-ticker"))
            .join(CONFIG_FILE),
    };
    let dir = config.parent().map(Path::to_path_buf).unwrap_or_default();
    DATA_DIR.set(dir).expect("paths::init is called once");
    config
}

pub fn data_dir() -> &'static Path {
    DATA_DIR.get().map(PathBuf::as_path).unwrap_or(Path::new(""))
}

/// `name` inside the data folder.
pub fn file(name: &str) -> PathBuf {
    data_dir().join(name)
}

/// A path from the config: absolute as is, relative against the data folder.
pub fn resolve(p: &str) -> PathBuf {
    let p = Path::new(p);
    if p.is_absolute() { p.to_path_buf() } else { data_dir().join(p) }
}

/// Files older versions kept in the current directory.
const LEGACY_FILES: &[&str] = &[
    CONFIG_FILE,
    "request-source.toml",
    "song-requests.json",
    "catalog-cache.json",
    ".env",
];

/// First run with the per-user folder: if it has no config yet but the
/// current directory has one from an older version, copy that setup over.
/// The originals stay where they were and are no longer used.
pub fn migrate_from_cwd(config: &Path) {
    if config.exists() || !Path::new(CONFIG_FILE).exists() {
        return;
    }
    for name in LEGACY_FILES {
        let (from, to) = (Path::new(name), file(name));
        if !from.exists() || to.exists() {
            continue;
        }
        match std::fs::copy(from, &to) {
            Ok(_) => tracing::info!("Copied {name} from the current directory to {}", to.display()),
            Err(e) => tracing::warn!("Couldn't copy {name} to {}: {e}", to.display()),
        }
    }
    tracing::info!(
        "Now using {}; the copies in the current directory are no longer read",
        data_dir().display()
    );
}
