use color_eyre::eyre::WrapErr as _;
use std::path::PathBuf;

/// Initialize color_eyre for tests. Safe to call multiple times.
pub fn init_test() {
    let _ = color_eyre::install();
}

/// Test overlay context with automatic cleanup.
pub struct TestOverlay {
    pub overlay: loaf::overlay::OverlayFs,
    pub base_path: PathBuf,
    _temp_dir: tempfile::TempDir,
}

impl TestOverlay {
    /// Create a new test overlay with temporary base directory and database.
    pub fn new() -> color_eyre::Result<Self> {
        let temp_dir = tempfile::tempdir().wrap_err("failed to create temp directory")?;
        let base_path = temp_dir.path().join("base");
        std::fs::create_dir(&base_path).wrap_err("failed to create base directory")?;

        let db_path = temp_dir.path().join("test.loaf");
        let overlay = loaf::overlay::OverlayFs::new(&db_path, &base_path)
            .wrap_err("failed to create overlay")?;

        Ok(Self {
            overlay,
            base_path: base_path.clone(),
            _temp_dir: temp_dir,
        })
    }

    /// Get the root inode.
    pub fn root_id(&self) -> u64 {
        loaf::overlay::OverlayFs::ROOT_ID
    }

    /// Create a file in the base directory (simulates existing filesystem).
    #[allow(dead_code)] // used by some but not all test modules
    pub fn create_base_file(&self, name: &str, content: &[u8]) -> color_eyre::Result<()> {
        let path = self.base_path.join(name);
        std::fs::write(&path, content)
            .wrap_err_with(|| format!("failed to write base file {path:?}"))?;
        Ok(())
    }

    /// Create a directory in the base directory.
    #[allow(dead_code)] // used by some but not all test modules
    pub fn create_base_dir(&self, name: &str) -> color_eyre::Result<()> {
        let path = self.base_path.join(name);
        std::fs::create_dir(&path)
            .wrap_err_with(|| format!("failed to create base directory {path:?}"))?;
        Ok(())
    }
}
