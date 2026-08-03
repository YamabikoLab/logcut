use std::fs;
use std::ops::Deref;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) const LOG_DIRECTORY_MARKER: &str = ".logcut-directory";
pub(crate) const LOG_DIRECTORY_MARKER_CONTENT: &str = "logcut log directory\n";

pub(crate) fn prepare_log_directory(path: &Path) {
    fs::create_dir_all(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();

    let marker = path.join(LOG_DIRECTORY_MARKER);
    fs::write(&marker, LOG_DIRECTORY_MARKER_CONTENT).unwrap();
    fs::set_permissions(marker, fs::Permissions::from_mode(0o600)).unwrap();
}

pub(crate) struct TestDir {
    path: PathBuf,
}

impl TestDir {
    pub(crate) fn new(prefix: &str, name: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("{prefix}-{name}-{}-{unique}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }
}

impl Deref for TestDir {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        &self.path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
