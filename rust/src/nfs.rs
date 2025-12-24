use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use color_eyre::eyre::WrapErr as _;
use nfsserve::{nfs::*, tcp::NFSTcp as _, vfs::*};

use crate::db::ItemType;
use crate::overlay::OverlayFs;

/// NFS wrapper around OverlayFs with thread-safe access
/// Uses Mutex + spawn_blocking since rusqlite::Connection is not Send
#[derive(Clone)]
pub struct NfsOverlay {
    inner: Arc<Mutex<OverlayFs>>,
}

impl NfsOverlay {
    pub fn new(overlay: OverlayFs) -> Self {
        Self {
            inner: Arc::new(Mutex::new(overlay)),
        }
    }

    /// Convert ItemType to ftype3
    fn item_type_to_ftype(item_type: ItemType) -> ftype3 {
        match item_type {
            ItemType::File => ftype3::NF3REG,
            ItemType::Directory => ftype3::NF3DIR,
            ItemType::Symlink => ftype3::NF3LNK,
        }
    }

    /// Convert Attrs to fattr3
    fn attrs_to_fattr3(attrs: &crate::db::Attrs) -> fattr3 {
        let ftype = Self::item_type_to_ftype(attrs.item_type);

        // Add file type bits to mode
        let mode_with_type = match attrs.item_type {
            ItemType::File => attrs.mode | 0o100000,      // S_IFREG
            ItemType::Directory => attrs.mode | 0o040000, // S_IFDIR
            ItemType::Symlink => attrs.mode | 0o120000,   // S_IFLNK
        };

        fattr3 {
            ftype,
            mode: mode_with_type,
            nlink: attrs.link_count,
            uid: attrs.uid,
            gid: attrs.gid,
            size: attrs.size,
            used: attrs.size,
            rdev: specdata3::default(),
            fsid: 1,
            fileid: attrs.file_id,
            atime: nfstime3 {
                seconds: attrs.atime_sec as u32,
                nseconds: attrs.atime_nsec as u32,
            },
            mtime: nfstime3 {
                seconds: attrs.mtime_sec as u32,
                nseconds: attrs.mtime_nsec as u32,
            },
            ctime: nfstime3 {
                seconds: attrs.ctime_sec as u32,
                nseconds: attrs.ctime_nsec as u32,
            },
        }
    }

    /// Convert filename3 (Vec<u8>) to String
    fn filename_to_str(filename: &filename3) -> Result<&str, nfsstat3> {
        std::str::from_utf8(filename).map_err(|_| nfsstat3::NFS3ERR_INVAL)
    }

    /// Convert nfspath3 (Vec<u8>) to String
    fn nfspath_to_string(path: &nfspath3) -> Result<String, nfsstat3> {
        std::str::from_utf8(path)
            .map(|s| s.to_string())
            .map_err(|_| nfsstat3::NFS3ERR_INVAL)
    }

    /// Map color_eyre errors to nfsstat3
    fn map_err(_err: color_eyre::Report) -> nfsstat3 {
        // For now, map all errors to NFSERR_IO
        // In production, you'd want more specific error mapping
        nfsstat3::NFS3ERR_IO
    }
}

#[async_trait]
impl NFSFileSystem for NfsOverlay {
    fn capabilities(&self) -> VFSCapabilities {
        VFSCapabilities::ReadWrite
    }

    fn root_dir(&self) -> fileid3 {
        1
    }

    async fn lookup(&self, dirid: fileid3, filename: &filename3) -> Result<fileid3, nfsstat3> {
        let name = Self::filename_to_str(filename)?.to_string();
        let inner = Arc::clone(&self.inner);

        tokio::task::spawn_blocking(move || {
            let mut overlay = inner.lock().unwrap();
            overlay.lookup(dirid, &name).map_err(|e| {
                tracing::debug!("lookup failed for {}/{}: {}", dirid, name, e);
                if e.to_string().contains("not found") || e.to_string().contains("whited out") {
                    nfsstat3::NFS3ERR_NOENT
                } else {
                    Self::map_err(e)
                }
            })
        })
        .await
        .unwrap()
    }

    async fn getattr(&self, id: fileid3) -> Result<fattr3, nfsstat3> {
        let inner = Arc::clone(&self.inner);

        tokio::task::spawn_blocking(move || {
            let overlay = inner.lock().unwrap();
            let attrs = overlay.getattr(id).map_err(|e| {
                tracing::debug!("getattr failed for {}: {}", id, e);
                if e.to_string().contains("not found") || e.to_string().contains("whited out") {
                    nfsstat3::NFS3ERR_NOENT
                } else {
                    Self::map_err(e)
                }
            })?;

            Ok(Self::attrs_to_fattr3(&attrs))
        })
        .await
        .unwrap()
    }

    async fn setattr(&self, id: fileid3, setattr: sattr3) -> Result<fattr3, nfsstat3> {
        let inner = Arc::clone(&self.inner);

        tokio::task::spawn_blocking(move || {
            let mut overlay = inner.lock().unwrap();

            let mode = match setattr.mode {
                set_mode3::mode(m) => Some(m),
                set_mode3::Void => None,
            };

            let size = match setattr.size {
                set_size3::size(s) => Some(s),
                set_size3::Void => None,
            };

            let atime = match setattr.atime {
                set_atime::SET_TO_CLIENT_TIME(t) => Some((t.seconds as i64, t.nseconds as i64)),
                set_atime::SET_TO_SERVER_TIME => {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap();
                    Some((now.as_secs() as i64, now.subsec_nanos() as i64))
                }
                set_atime::DONT_CHANGE => None,
            };

            let mtime = match setattr.mtime {
                set_mtime::SET_TO_CLIENT_TIME(t) => Some((t.seconds as i64, t.nseconds as i64)),
                set_mtime::SET_TO_SERVER_TIME => {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap();
                    Some((now.as_secs() as i64, now.subsec_nanos() as i64))
                }
                set_mtime::DONT_CHANGE => None,
            };

            overlay
                .setattr(id, mode, size, atime, mtime)
                .map_err(|e| {
                    tracing::error!("setattr failed for {}: {}", id, e);
                    if e.to_string().contains("not found") {
                        nfsstat3::NFS3ERR_NOENT
                    } else {
                        Self::map_err(e)
                    }
                })?;

            // Return updated attributes
            let attrs = overlay.getattr(id).map_err(Self::map_err)?;
            Ok(Self::attrs_to_fattr3(&attrs))
        })
        .await
        .unwrap()
    }

    async fn read(&self, id: fileid3, offset: u64, count: u32) -> Result<(Vec<u8>, bool), nfsstat3> {
        let inner = Arc::clone(&self.inner);

        tokio::task::spawn_blocking(move || {
            let overlay = inner.lock().unwrap();

            let mut buf = vec![0u8; count as usize];
            let n = overlay.read(id, offset, &mut buf).map_err(|e| {
                tracing::debug!("read failed for {}: {}", id, e);
                if e.to_string().contains("not found") || e.to_string().contains("whited out") {
                    nfsstat3::NFS3ERR_NOENT
                } else {
                    Self::map_err(e)
                }
            })?;

            buf.truncate(n);
            let eof = n < count as usize;
            Ok((buf, eof))
        })
        .await
        .unwrap()
    }

    async fn write(&self, id: fileid3, offset: u64, data: &[u8]) -> Result<fattr3, nfsstat3> {
        let data = data.to_vec();
        let inner = Arc::clone(&self.inner);

        tokio::task::spawn_blocking(move || {
            let mut overlay = inner.lock().unwrap();

            overlay.write(id, offset, &data).map_err(|e| {
                tracing::error!("write failed for {}: {}", id, e);
                if e.to_string().contains("not found") {
                    nfsstat3::NFS3ERR_NOENT
                } else {
                    Self::map_err(e)
                }
            })?;

            // Return updated attributes
            let attrs = overlay.getattr(id).map_err(Self::map_err)?;
            Ok(Self::attrs_to_fattr3(&attrs))
        })
        .await
        .unwrap()
    }

    async fn create(
        &self,
        dirid: fileid3,
        filename: &filename3,
        attr: sattr3,
    ) -> Result<(fileid3, fattr3), nfsstat3> {
        let name = Self::filename_to_str(filename)?.to_string();
        let inner = Arc::clone(&self.inner);

        tokio::task::spawn_blocking(move || {
            let mut overlay = inner.lock().unwrap();

            let mode = match attr.mode {
                set_mode3::mode(m) => m & 0o777,
                set_mode3::Void => 0o644,
            };

            let inode = overlay
                .create(dirid, &name, ItemType::File, mode)
                .map_err(|e| {
                    tracing::error!("create failed for {}/{}: {}", dirid, name, e);
                    if e.to_string().contains("not found") {
                        nfsstat3::NFS3ERR_NOENT
                    } else {
                        Self::map_err(e)
                    }
                })?;

            let attrs = overlay.getattr(inode).map_err(Self::map_err)?;
            Ok((inode, Self::attrs_to_fattr3(&attrs)))
        })
        .await
        .unwrap()
    }

    async fn create_exclusive(
        &self,
        dirid: fileid3,
        filename: &filename3,
    ) -> Result<fileid3, nfsstat3> {
        let name = Self::filename_to_str(filename)?.to_string();
        let inner = Arc::clone(&self.inner);

        tokio::task::spawn_blocking(move || {
            let mut overlay = inner.lock().unwrap();

            // Check if file already exists
            if overlay.lookup(dirid, &name).is_ok() {
                return Err(nfsstat3::NFS3ERR_EXIST);
            }

            let inode = overlay
                .create(dirid, &name, ItemType::File, 0o644)
                .map_err(|e| {
                    tracing::error!("create_exclusive failed for {}/{}: {}", dirid, name, e);
                    Self::map_err(e)
                })?;

            Ok(inode)
        })
        .await
        .unwrap()
    }

    async fn mkdir(
        &self,
        dirid: fileid3,
        dirname: &filename3,
    ) -> Result<(fileid3, fattr3), nfsstat3> {
        let name = Self::filename_to_str(dirname)?.to_string();
        let inner = Arc::clone(&self.inner);

        tokio::task::spawn_blocking(move || {
            let mut overlay = inner.lock().unwrap();

            let inode = overlay.mkdir(dirid, &name, 0o755).map_err(|e| {
                tracing::error!("mkdir failed for {}/{}: {}", dirid, name, e);
                if e.to_string().contains("not found") {
                    nfsstat3::NFS3ERR_NOENT
                } else {
                    Self::map_err(e)
                }
            })?;

            let attrs = overlay.getattr(inode).map_err(Self::map_err)?;
            Ok((inode, Self::attrs_to_fattr3(&attrs)))
        })
        .await
        .unwrap()
    }

    async fn remove(&self, dirid: fileid3, filename: &filename3) -> Result<(), nfsstat3> {
        let name = Self::filename_to_str(filename)?.to_string();
        let inner = Arc::clone(&self.inner);

        tokio::task::spawn_blocking(move || {
            let mut overlay = inner.lock().unwrap();

            let inode = overlay.lookup(dirid, &name).map_err(|_| nfsstat3::NFS3ERR_NOENT)?;

            overlay.remove(inode).map_err(|e| {
                tracing::error!("remove failed for {}/{}: {}", dirid, name, e);
                Self::map_err(e)
            })?;

            Ok(())
        })
        .await
        .unwrap()
    }

    async fn rename(
        &self,
        from_dirid: fileid3,
        from_filename: &filename3,
        to_dirid: fileid3,
        to_filename: &filename3,
    ) -> Result<(), nfsstat3> {
        let from_name = Self::filename_to_str(from_filename)?.to_string();
        let to_name = Self::filename_to_str(to_filename)?.to_string();
        let inner = Arc::clone(&self.inner);

        tokio::task::spawn_blocking(move || {
            let mut overlay = inner.lock().unwrap();

            overlay
                .rename(from_dirid, &from_name, to_dirid, &to_name)
                .map_err(|e| {
                    tracing::error!(
                        "rename failed from {}/{} to {}/{}: {}",
                        from_dirid,
                        from_name,
                        to_dirid,
                        to_name,
                        e
                    );
                    if e.to_string().contains("not found") {
                        nfsstat3::NFS3ERR_NOENT
                    } else {
                        Self::map_err(e)
                    }
                })?;

            Ok(())
        })
        .await
        .unwrap()
    }

    async fn readdir(
        &self,
        dirid: fileid3,
        start_after: fileid3,
        max_entries: usize,
    ) -> Result<ReadDirResult, nfsstat3> {
        let inner = Arc::clone(&self.inner);

        tokio::task::spawn_blocking(move || {
            let mut overlay = inner.lock().unwrap();

            let entries = overlay.readdir(dirid).map_err(|e| {
                tracing::debug!("readdir failed for {}: {}", dirid, e);
                if e.to_string().contains("not found") || e.to_string().contains("whited out") {
                    nfsstat3::NFS3ERR_NOENT
                } else {
                    Self::map_err(e)
                }
            })?;

            // Filter entries to start after the specified inode
            let mut result_entries = Vec::new();
            let mut started = start_after == 0;

            for entry in entries {
                if !started {
                    if entry.inode_id == start_after {
                        started = true;
                    }
                    continue;
                }

                if result_entries.len() >= max_entries {
                    break;
                }

                // Get attributes for this entry
                let attrs = overlay.getattr(entry.inode_id).map_err(Self::map_err)?;

                result_entries.push(DirEntry {
                    fileid: entry.inode_id,
                    name: entry.name.into_bytes().into(),
                    attr: Self::attrs_to_fattr3(&attrs),
                });
            }

            let end = result_entries.len() < max_entries;

            Ok(ReadDirResult {
                entries: result_entries,
                end,
            })
        })
        .await
        .unwrap()
    }

    async fn symlink(
        &self,
        dirid: fileid3,
        linkname: &filename3,
        symlink: &nfspath3,
        _attr: &sattr3,
    ) -> Result<(fileid3, fattr3), nfsstat3> {
        let name = Self::filename_to_str(linkname)?.to_string();
        let target = Self::nfspath_to_string(symlink)?;
        let inner = Arc::clone(&self.inner);

        tokio::task::spawn_blocking(move || {
            let mut overlay = inner.lock().unwrap();

            let inode = overlay.symlink(dirid, &name, &target).map_err(|e| {
                tracing::error!("symlink failed for {}/{}: {}", dirid, name, e);
                if e.to_string().contains("not found") {
                    nfsstat3::NFS3ERR_NOENT
                } else {
                    Self::map_err(e)
                }
            })?;

            let attrs = overlay.getattr(inode).map_err(Self::map_err)?;
            Ok((inode, Self::attrs_to_fattr3(&attrs)))
        })
        .await
        .unwrap()
    }

    async fn readlink(&self, id: fileid3) -> Result<nfspath3, nfsstat3> {
        let inner = Arc::clone(&self.inner);

        tokio::task::spawn_blocking(move || {
            let overlay = inner.lock().unwrap();

            let target = overlay.readlink(id).map_err(|e| {
                tracing::debug!("readlink failed for {}: {}", id, e);
                if e.to_string().contains("not found") || e.to_string().contains("whited out") {
                    nfsstat3::NFS3ERR_NOENT
                } else {
                    Self::map_err(e)
                }
            })?;

            Ok(target.into_bytes().into())
        })
        .await
        .unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_test_nfs() -> color_eyre::Result<(NfsOverlay, tempfile::TempDir)> {
        let temp_dir = tempfile::tempdir()?;
        let db_path = temp_dir.path().join("test.loaf");
        let base_path = temp_dir.path().join("base");
        std::fs::create_dir(&base_path)?;

        let overlay = OverlayFs::new(&db_path, &base_path)?;
        let nfs = NfsOverlay::new(overlay);
        Ok((nfs, temp_dir))
    }

    #[tokio::test]
    async fn test_nfs_root_dir() -> color_eyre::Result<()> {
        let (nfs, _temp) = setup_test_nfs().await?;
        assert_eq!(nfs.root_dir(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn test_nfs_capabilities() -> color_eyre::Result<()> {
        let (nfs, _temp) = setup_test_nfs().await?;
        matches!(nfs.capabilities(), VFSCapabilities::ReadWrite);
        Ok(())
    }

    #[tokio::test]
    async fn test_nfs_getattr_root() -> color_eyre::Result<()> {
        let (nfs, _temp) = setup_test_nfs().await?;
        let attrs = nfs.getattr(1).await?;
        assert_eq!(attrs.fileid, 1);
        assert_eq!(attrs.ftype, ftype3::NF3DIR);
        Ok(())
    }

    #[tokio::test]
    async fn test_nfs_create_and_lookup() -> color_eyre::Result<()> {
        let (nfs, _temp) = setup_test_nfs().await?;

        let filename = b"test.txt".to_vec();
        let (file_id, attrs) = nfs
            .create(1, &filename, sattr3::default())
            .await?;

        assert_eq!(attrs.ftype, ftype3::NF3REG);

        let looked_up = nfs.lookup(1, &filename).await?;
        assert_eq!(looked_up, file_id);

        Ok(())
    }

    #[tokio::test]
    async fn test_nfs_write_and_read() -> color_eyre::Result<()> {
        let (nfs, _temp) = setup_test_nfs().await?;

        let filename = b"test.txt".to_vec();
        let (file_id, _) = nfs.create(1, &filename, sattr3::default()).await?;

        let data = b"hello world";
        nfs.write(file_id, 0, data).await?;

        let (read_data, eof) = nfs.read(file_id, 0, 100).await?;
        assert_eq!(read_data, data);
        assert!(eof);

        Ok(())
    }

    #[tokio::test]
    async fn test_nfs_mkdir_and_readdir() -> color_eyre::Result<()> {
        let (nfs, _temp) = setup_test_nfs().await?;

        let dirname = b"testdir".to_vec();
        let (dir_id, attrs) = nfs.mkdir(1, &dirname).await?;

        assert_eq!(attrs.ftype, ftype3::NF3DIR);

        let result = nfs.readdir(1, 0, 10).await?;
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].fileid, dir_id);
        assert_eq!(result.entries[0].name, dirname);

        Ok(())
    }

    #[tokio::test]
    async fn test_nfs_symlink_and_readlink() -> color_eyre::Result<()> {
        let (nfs, _temp) = setup_test_nfs().await?;

        let linkname = b"link".to_vec();
        let target = b"/target/path".to_vec();
        let (link_id, attrs) = nfs
            .symlink(1, &linkname, &target, &sattr3::default())
            .await?;

        assert_eq!(attrs.ftype, ftype3::NF3LNK);

        let read_target = nfs.readlink(link_id).await?;
        assert_eq!(read_target, target);

        Ok(())
    }

    #[tokio::test]
    async fn test_nfs_rename() -> color_eyre::Result<()> {
        let (nfs, _temp) = setup_test_nfs().await?;

        let old_name = b"old.txt".to_vec();
        let new_name = b"new.txt".to_vec();

        let (file_id, _) = nfs.create(1, &old_name, sattr3::default()).await?;

        nfs.rename(1, &old_name, 1, &new_name).await?;

        assert!(nfs.lookup(1, &old_name).await.is_err());
        let new_id = nfs.lookup(1, &new_name).await?;
        assert_eq!(new_id, file_id);

        Ok(())
    }

    #[tokio::test]
    async fn test_nfs_remove() -> color_eyre::Result<()> {
        let (nfs, _temp) = setup_test_nfs().await?;

        let filename = b"test.txt".to_vec();
        nfs.create(1, &filename, sattr3::default()).await?;

        nfs.remove(1, &filename).await?;

        assert!(nfs.lookup(1, &filename).await.is_err());

        Ok(())
    }

    #[tokio::test]
    async fn test_nfs_setattr() -> color_eyre::Result<()> {
        let (nfs, _temp) = setup_test_nfs().await?;

        let filename = b"test.txt".to_vec();
        let (file_id, _) = nfs.create(1, &filename, sattr3::default()).await?;

        let new_mode = sattr3 {
            mode: set_mode3::mode(0o600),
            ..Default::default()
        };

        let attrs = nfs.setattr(file_id, new_mode).await?;
        assert_eq!(attrs.mode & 0o777, 0o600);

        Ok(())
    }
}

/// NFS server configuration
pub struct NfsServer {
    pub port: u16,
    #[allow(dead_code)]
    pub overlay: NfsOverlay,
}

impl NfsServer {
    /// Start NFS server on a random port (or specified port if provided)
    /// Returns the actual port bound and the server task handle
    pub async fn start(overlay: OverlayFs, port: Option<u16>) -> color_eyre::Result<(Self, tokio::task::JoinHandle<()>)> {
        let nfs_overlay = NfsOverlay::new(overlay);
        let listener = nfsserve::tcp::NFSTcpListener::bind(
            &format!("127.0.0.1:{}", port.unwrap_or(0)),
            nfs_overlay.clone(),
        )
        .await
        .map_err(|e| color_eyre::eyre::eyre!("failed to bind NFS server: {}", e))?;

        let actual_port = listener.get_listen_port();
        tracing::info!("NFS server listening on 127.0.0.1:{}", actual_port);

        let server = Self {
            port: actual_port,
            overlay: nfs_overlay,
        };

        // Spawn server task
        let handle = tokio::spawn(async move {
            if let Err(e) = listener.handle_forever().await {
                tracing::error!("NFS server error: {}", e);
            }
        });

        Ok((server, handle))
    }
}

/// Mount NFS filesystem via mount_nfs command
pub async fn mount_nfs(port: u16, mount_point: &std::path::Path) -> color_eyre::Result<()> {
    use tokio::process::Command;

    // Create mount point if it doesn't exist
    tokio::fs::create_dir_all(mount_point)
        .await
        .wrap_err_with(|| format!("failed to create mount point {mount_point:?}"))?;

    let mount_opts = format!(
        "nolocks,vers=3,tcp,rsize=131072,port={port},mountport={port}"
    );

    tracing::debug!("executing: mount_nfs -o {mount_opts} localhost:/ {mount_point:?}");

    // Add timeout to prevent hanging indefinitely
    let mount_future = Command::new("mount_nfs")
        .arg("-o")
        .arg(&mount_opts)
        .arg(format!("localhost:/"))
        .arg(mount_point)
        .output();

    let output = tokio::time::timeout(std::time::Duration::from_secs(30), mount_future)
        .await
        .wrap_err_with(|| {
            format!(
                "mount_nfs command timed out after 30 seconds\n\
                 This usually means:\n\
                 - NFS server is not responding on port {port}\n\
                 - Firewall is blocking localhost:{port}\n\
                 - Another process is already using port {port}"
            )
        })?
        .wrap_err("failed to execute mount_nfs command")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);

        // Provide helpful error messages based on common failure modes
        let hint = if stderr.contains("Permission denied") || stderr.contains("Operation not permitted") {
            "\nHint: Try running with sudo or check NFS permissions"
        } else if stderr.contains("already mounted") || stderr.contains("busy") {
            "\nHint: Directory is already mounted. Unmount first with: loaf unmount"
        } else if stderr.contains("Connection refused") || stderr.contains("RPC") {
            "\nHint: NFS server may not be running or port is in use"
        } else {
            ""
        };

        color_eyre::eyre::bail!(
            "mount_nfs failed (exit code {}):\n{}{}\n{}",
            output.status,
            stderr.trim(),
            if !stdout.is_empty() { format!("\n{}", stdout.trim()) } else { String::new() },
            hint
        );
    }

    tracing::info!("Mounted NFS at {mount_point:?}");
    Ok(())
}

/// Unmount NFS filesystem via umount command
pub async fn unmount_nfs(mount_point: &std::path::Path) -> color_eyre::Result<()> {
    use tokio::process::Command;

    tracing::debug!("executing: umount {mount_point:?}");

    let output = Command::new("umount")
        .arg(mount_point)
        .output()
        .await
        .wrap_err("failed to execute umount command")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);

        // Provide helpful error messages based on common failure modes
        let hint = if stderr.contains("Permission denied") || stderr.contains("Operation not permitted") {
            "\nHint: Try running with sudo: sudo loaf unmount"
        } else if stderr.contains("not currently mounted") || stderr.contains("not a mount point") {
            "\nHint: Directory is not currently mounted"
        } else if stderr.contains("busy") || stderr.contains("in use") {
            "\nHint: Files may be in use. Close any programs accessing the mount point and try again"
        } else {
            ""
        };

        color_eyre::eyre::bail!(
            "umount failed (exit code {}):\n{}{}\n{}",
            output.status,
            stderr.trim(),
            if !stdout.is_empty() { format!("\n{}", stdout.trim()) } else { String::new() },
            hint
        );
    }

    tracing::info!("Unmounted NFS at {mount_point:?}");
    Ok(())
}
