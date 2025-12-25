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
    pub db_path: PathBuf,
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
            db_path: db_path.clone(),
            _temp_dir: temp_dir,
        })
    }

    /// Get the root inode.
    pub fn root_id(&self) -> u64 {
        self.overlay.root_id()
    }

    /// Create a file in the base directory (simulates existing filesystem).
    pub fn create_base_file(&self, name: &str, content: &[u8]) -> color_eyre::Result<()> {
        let path = self.base_path.join(name);
        std::fs::write(&path, content)
            .wrap_err_with(|| format!("failed to write base file {path:?}"))?;
        Ok(())
    }

    /// Create a directory in the base directory.
    pub fn create_base_dir(&self, name: &str) -> color_eyre::Result<()> {
        let path = self.base_path.join(name);
        std::fs::create_dir(&path)
            .wrap_err_with(|| format!("failed to create base directory {path:?}"))?;
        Ok(())
    }
}

/// Run a shell command and capture output.
pub fn run_command(
    program: &str,
    args: &[&str],
    cwd: &std::path::Path,
) -> color_eyre::Result<std::process::Output> {
    let output = std::process::Command::new(program)
        .args(args)
        .current_dir(cwd)
        .output()
        .wrap_err_with(|| format!("failed to execute {program}"))?;

    Ok(output)
}

/// Run a shell command and return stdout as string.
pub fn run_command_stdout(
    program: &str,
    args: &[&str],
    cwd: &std::path::Path,
) -> color_eyre::Result<String> {
    let output = run_command(program, args, cwd)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        color_eyre::eyre::bail!(
            "{program} failed with exit code {:?}: {stderr}",
            output.status.code()
        );
    }

    Ok(String::from_utf8(output.stdout).wrap_err("command output is not valid UTF-8")?)
}

/// Git environment variables for deterministic commits.
pub struct GitEnv {
    vars: Vec<(&'static str, &'static str)>,
}

impl GitEnv {
    pub fn new() -> Self {
        Self {
            vars: vec![
                ("GIT_AUTHOR_NAME", "Test User"),
                ("GIT_AUTHOR_EMAIL", "test@example.com"),
                ("GIT_COMMITTER_NAME", "Test User"),
                ("GIT_COMMITTER_EMAIL", "test@example.com"),
            ],
        }
    }

    /// Apply environment variables to a Command.
    pub fn apply(&self, cmd: &mut std::process::Command) {
        for (key, value) in &self.vars {
            cmd.env(key, value);
        }
    }
}

/// Run git command with test environment variables.
pub fn run_git(args: &[&str], cwd: &std::path::Path) -> color_eyre::Result<std::process::Output> {
    let git_env = GitEnv::new();
    let mut cmd = std::process::Command::new("git");
    cmd.args(args).current_dir(cwd);
    git_env.apply(&mut cmd);

    let output = cmd.output().wrap_err("failed to execute git")?;

    Ok(output)
}

/// Run git command and return stdout as string.
pub fn run_git_stdout(args: &[&str], cwd: &std::path::Path) -> color_eyre::Result<String> {
    let output = run_git(args, cwd)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        color_eyre::eyre::bail!("git {} failed: {stderr}", args.join(" "));
    }

    Ok(String::from_utf8(output.stdout).wrap_err("git output is not valid UTF-8")?)
}
