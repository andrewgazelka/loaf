use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::db::{Attrs, Database, DirEntry, ItemType};

pub struct OverlayFs {
    db: Database,
    base_path: PathBuf,
    inode_to_path: HashMap<u64, String>,
    path_to_inode: HashMap<String, u64>,
    next_inode: u64,
}

impl OverlayFs {
    pub fn new(db_path: &Path, base_path: &Path) -> color_eyre::Result<Self> {
        let db = Database::open(db_path)?;
        db.set_base_path(
            base_path
                .to_str()
                .ok_or_else(|| color_eyre::eyre::eyre!("invalid base path"))?,
        )?;

        let mut overlay = Self {
            db,
            base_path: base_path.to_path_buf(),
            inode_to_path: HashMap::new(),
            path_to_inode: HashMap::new(),
            next_inode: 1_000_000,
        };

        overlay.register_path(1, "/".to_string());

        Ok(overlay)
    }

    fn register_path(&mut self, inode: u64, path: String) {
        self.path_to_inode.insert(path.clone(), inode);
        self.inode_to_path.insert(inode, path);
    }

    fn get_path(&self, inode: u64) -> Option<&str> {
        self.inode_to_path.get(&inode).map(|s| s.as_str())
    }

    fn get_or_create_inode(&mut self, path: &str) -> u64 {
        if let Some(&inode) = self.path_to_inode.get(path) {
            return inode;
        }
        let inode = self.next_inode;
        self.next_inode += 1;
        self.register_path(inode, path.to_string());
        inode
    }

    fn join_path(parent: &str, name: &str) -> String {
        if parent == "/" {
            format!("/{name}")
        } else {
            format!("{parent}/{name}")
        }
    }

    fn real_path(&self, rel_path: &str) -> PathBuf {
        if rel_path == "/" {
            return self.base_path.clone();
        }
        let trimmed = rel_path.strip_prefix('/').unwrap_or(rel_path);
        self.base_path.join(trimmed)
    }

    pub fn root_id(&self) -> u64 {
        1
    }

    pub fn getattr(&self, inode: u64) -> color_eyre::Result<Attrs> {
        let path = self
            .get_path(inode)
            .ok_or_else(|| color_eyre::eyre::eyre!("inode {inode} not found"))?;

        if self.db.is_whiteout(path) {
            color_eyre::eyre::bail!("path {path:?} is whited out");
        }

        if let Ok(attrs) = self.db.get_attrs_by_path(path) {
            return Ok(attrs);
        }

        let real = self.real_path(path);
        let meta = std::fs::metadata(&real)
            .map_err(|e| color_eyre::eyre::eyre!("failed to stat {real:?}: {e}"))?;

        let item_type = if meta.is_dir() {
            ItemType::Directory
        } else if meta.file_type().is_symlink() {
            ItemType::Symlink
        } else {
            ItemType::File
        };

        use std::os::unix::fs::MetadataExt as _;
        Ok(Attrs {
            file_id: inode,
            parent_id: 0,
            item_type,
            mode: meta.mode(),
            uid: meta.uid(),
            gid: meta.gid(),
            size: meta.size(),
            link_count: meta.nlink() as u32,
            atime_sec: meta.atime(),
            atime_nsec: meta.atime_nsec(),
            mtime_sec: meta.mtime(),
            mtime_nsec: meta.mtime_nsec(),
            ctime_sec: meta.ctime(),
            ctime_nsec: meta.ctime_nsec(),
        })
    }

    pub fn lookup(&mut self, parent_id: u64, name: &str) -> color_eyre::Result<u64> {
        let parent_path = self
            .get_path(parent_id)
            .ok_or_else(|| color_eyre::eyre::eyre!("parent inode {parent_id} not found"))?
            .to_string();

        let child_path = Self::join_path(&parent_path, name);

        if self.db.is_whiteout(&child_path) {
            color_eyre::eyre::bail!("path {child_path:?} is whited out");
        }

        if self.db.exists_by_path(&child_path) {
            return Ok(self.get_or_create_inode(&child_path));
        }

        let real = self.real_path(&child_path);
        if real.exists() {
            return Ok(self.get_or_create_inode(&child_path));
        }

        color_eyre::eyre::bail!("not found: {child_path:?}")
    }

    pub fn create(&mut self, parent_id: u64, name: &str, item_type: ItemType, mode: u32) -> color_eyre::Result<u64> {
        let parent_path = self
            .get_path(parent_id)
            .ok_or_else(|| color_eyre::eyre::eyre!("parent inode {parent_id} not found"))?
            .to_string();

        let child_path = Self::join_path(&parent_path, name);

        self.db.remove_whiteout(&child_path)?;
        self.db.create_by_path(&child_path, item_type, mode)?;

        Ok(self.get_or_create_inode(&child_path))
    }

    pub fn read(&self, inode: u64, offset: u64, buf: &mut [u8]) -> color_eyre::Result<usize> {
        let path = self
            .get_path(inode)
            .ok_or_else(|| color_eyre::eyre::eyre!("inode {inode} not found"))?;

        if self.db.is_whiteout(path) {
            color_eyre::eyre::bail!("path {path:?} is whited out");
        }

        if let Ok(n) = self.db.read_by_path(path, offset, buf) {
            return Ok(n);
        }

        let real = self.real_path(path);
        use std::io::{Read as _, Seek as _, SeekFrom};
        let mut file = std::fs::File::open(&real)
            .map_err(|e| color_eyre::eyre::eyre!("failed to open {real:?}: {e}"))?;

        if offset > 0 {
            file.seek(SeekFrom::Start(offset))?;
        }

        let n = file.read(buf)?;
        Ok(n)
    }

    pub fn write(&mut self, inode: u64, offset: u64, data: &[u8]) -> color_eyre::Result<usize> {
        let path = self
            .get_path(inode)
            .ok_or_else(|| color_eyre::eyre::eyre!("inode {inode} not found"))?
            .to_string();

        if !self.db.exists_by_path(&path) {
            self.db.create_by_path(&path, ItemType::File, 0o644)?;

            let real = self.real_path(&path);
            if let Ok(content) = std::fs::read(&real) {
                self.db.write_by_path(&path, 0, &content)?;
            }
        }

        self.db.write_by_path(&path, offset, data)
    }

    pub fn remove(&mut self, inode: u64) -> color_eyre::Result<()> {
        let path = self
            .get_path(inode)
            .ok_or_else(|| color_eyre::eyre::eyre!("inode {inode} not found"))?
            .to_string();

        if self.db.exists_by_path(&path) {
            self.db.remove_by_path(&path)?;
        }

        let real = self.real_path(&path);
        if real.exists() {
            self.db.add_whiteout(&path)?;
        }

        Ok(())
    }

    pub fn readlink(&self, inode: u64) -> color_eyre::Result<String> {
        let path = self
            .get_path(inode)
            .ok_or_else(|| color_eyre::eyre::eyre!("inode {inode} not found"))?;

        if let Ok(target) = self.db.read_symlink_by_path(path) {
            return Ok(target);
        }

        let real = self.real_path(path);
        let target = std::fs::read_link(&real)
            .map_err(|e| color_eyre::eyre::eyre!("failed to read symlink {real:?}: {e}"))?;

        target
            .to_str()
            .map(|s| s.to_string())
            .ok_or_else(|| color_eyre::eyre::eyre!("symlink target is not valid UTF-8"))
    }

    pub fn rename(&mut self, old_parent_id: u64, old_name: &str, new_parent_id: u64, new_name: &str) -> color_eyre::Result<()> {
        let old_parent_path = self
            .get_path(old_parent_id)
            .ok_or_else(|| color_eyre::eyre::eyre!("old parent inode {old_parent_id} not found"))?
            .to_string();

        let new_parent_path = self
            .get_path(new_parent_id)
            .ok_or_else(|| color_eyre::eyre::eyre!("new parent inode {new_parent_id} not found"))?
            .to_string();

        let old_path = Self::join_path(&old_parent_path, old_name);
        let new_path = Self::join_path(&new_parent_path, new_name);

        if !self.db.exists_by_path(&old_path) {
            let real = self.real_path(&old_path);
            if !real.exists() {
                color_eyre::eyre::bail!("source path {old_path:?} does not exist");
            }

            let meta = std::fs::metadata(&real)
                .map_err(|e| color_eyre::eyre::eyre!("failed to stat {real:?}: {e}"))?;

            let item_type = if meta.is_dir() {
                ItemType::Directory
            } else if meta.file_type().is_symlink() {
                ItemType::Symlink
            } else {
                ItemType::File
            };

            use std::os::unix::fs::MetadataExt as _;
            self.db.create_by_path(&old_path, item_type, meta.mode())?;

            if item_type == ItemType::File {
                let content = std::fs::read(&real)?;
                self.db.write_by_path(&old_path, 0, &content)?;
            } else if item_type == ItemType::Symlink {
                let target = std::fs::read_link(&real)?;
                let target_str = target.to_str()
                    .ok_or_else(|| color_eyre::eyre::eyre!("symlink target is not valid UTF-8"))?;
                self.db.create_symlink_by_path(&old_path, target_str)?;
            }
        }

        self.db.rename_by_path(&old_path, &new_path)?;

        if let Some(&old_inode) = self.path_to_inode.get(&old_path) {
            self.path_to_inode.remove(&old_path);
            self.path_to_inode.insert(new_path.clone(), old_inode);
            self.inode_to_path.insert(old_inode, new_path);
        }

        Ok(())
    }

    pub fn setattr(&mut self, inode: u64, mode: Option<u32>, size: Option<u64>, atime: Option<(i64, i64)>, mtime: Option<(i64, i64)>) -> color_eyre::Result<()> {
        let path = self
            .get_path(inode)
            .ok_or_else(|| color_eyre::eyre::eyre!("inode {inode} not found"))?
            .to_string();

        if !self.db.exists_by_path(&path) {
            let real = self.real_path(&path);
            if !real.exists() {
                color_eyre::eyre::bail!("path {path:?} does not exist");
            }

            let meta = std::fs::metadata(&real)
                .map_err(|e| color_eyre::eyre::eyre!("failed to stat {real:?}: {e}"))?;

            let item_type = if meta.is_dir() {
                ItemType::Directory
            } else if meta.file_type().is_symlink() {
                ItemType::Symlink
            } else {
                ItemType::File
            };

            use std::os::unix::fs::MetadataExt as _;
            self.db.create_by_path(&path, item_type, meta.mode())?;

            if item_type == ItemType::File {
                let content = std::fs::read(&real)?;
                self.db.write_by_path(&path, 0, &content)?;
            }
        }

        if let Some(new_size) = size {
            self.db.truncate_by_path(&path, new_size)?;
        }

        if let Some(m) = mode {
            self.db.update_mode_by_path(&path, m)?;
        }

        if atime.is_some() || mtime.is_some() {
            let (atime_sec, atime_nsec) = atime.unwrap_or_else(|| {
                let attrs = self.db.get_attrs_by_path(&path).unwrap();
                (attrs.atime_sec, attrs.atime_nsec)
            });

            let (mtime_sec, mtime_nsec) = mtime.unwrap_or_else(|| {
                let attrs = self.db.get_attrs_by_path(&path).unwrap();
                (attrs.mtime_sec, attrs.mtime_nsec)
            });

            self.db.update_times_by_path(&path, Some(atime_sec), Some(atime_nsec), Some(mtime_sec), Some(mtime_nsec))?;
        }

        Ok(())
    }

    pub fn mkdir(&mut self, parent_id: u64, name: &str, mode: u32) -> color_eyre::Result<u64> {
        self.create(parent_id, name, ItemType::Directory, mode)
    }

    pub fn symlink(&mut self, parent_id: u64, name: &str, target: &str) -> color_eyre::Result<u64> {
        let parent_path = self
            .get_path(parent_id)
            .ok_or_else(|| color_eyre::eyre::eyre!("parent inode {parent_id} not found"))?
            .to_string();

        let link_path = Self::join_path(&parent_path, name);

        self.db.remove_whiteout(&link_path)?;
        self.db.create_symlink_by_path(&link_path, target)?;

        Ok(self.get_or_create_inode(&link_path))
    }

    pub fn readdir(&mut self, dir_inode: u64) -> color_eyre::Result<Vec<DirEntry>> {
        let dir_path = self
            .get_path(dir_inode)
            .ok_or_else(|| color_eyre::eyre::eyre!("inode {dir_inode} not found"))?
            .to_string();

        let mut entries = Vec::new();
        let mut seen = std::collections::HashSet::new();

        if let Ok(db_entries) = self.db.list_children_by_parent_path(&dir_path) {
            for entry in db_entries {
                let child_path = Self::join_path(&dir_path, &entry.name);
                if !self.db.is_whiteout(&child_path) {
                    let inode = self.get_or_create_inode(&child_path);
                    entries.push(DirEntry {
                        inode_id: inode,
                        name: entry.name.clone(),
                        item_type: entry.item_type,
                    });
                    seen.insert(entry.name);
                }
            }
        }

        let real = self.real_path(&dir_path);
        if let Ok(read_dir) = std::fs::read_dir(&real) {
            for entry in read_dir.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                let child_path = Self::join_path(&dir_path, &name);

                if seen.contains(&name) || self.db.is_whiteout(&child_path) {
                    continue;
                }

                let meta = match entry.metadata() {
                    Ok(m) => m,
                    Err(_) => continue,
                };

                let item_type = if meta.is_dir() {
                    ItemType::Directory
                } else if meta.file_type().is_symlink() {
                    ItemType::Symlink
                } else {
                    ItemType::File
                };

                let inode = self.get_or_create_inode(&child_path);
                entries.push(DirEntry {
                    inode_id: inode,
                    name: name.clone(),
                    item_type,
                });
                seen.insert(name);
            }
        }

        Ok(entries)
    }

    pub fn get_all_inodes(&self) -> color_eyre::Result<Vec<(String, ItemType)>> {
        use color_eyre::eyre::WrapErr as _;

        let mut stmt = self.db.conn()
            .prepare("SELECT path, type FROM inodes WHERE id != 1")
            .wrap_err("failed to prepare get_all_inodes query")?;

        let inodes = stmt
            .query_map([], |row| {
                let path: String = row.get(0)?;
                let item_type = ItemType::try_from(row.get::<_, i64>(1)?).unwrap();
                Ok((path, item_type))
            })
            .wrap_err("failed to query all inodes")?
            .collect::<Result<Vec<_>, _>>()
            .wrap_err("failed to collect inodes")?;

        Ok(inodes)
    }

    pub fn get_all_whiteouts(&self) -> color_eyre::Result<Vec<String>> {
        use color_eyre::eyre::WrapErr as _;

        let mut stmt = self.db.conn()
            .prepare("SELECT path FROM whiteouts")
            .wrap_err("failed to prepare get_all_whiteouts query")?;

        let whiteouts = stmt
            .query_map([], |row| row.get(0))
            .wrap_err("failed to query all whiteouts")?
            .collect::<Result<Vec<_>, _>>()
            .wrap_err("failed to collect whiteouts")?;

        Ok(whiteouts)
    }

    pub fn base_path(&self) -> &Path {
        &self.base_path
    }

    pub fn read_file_data(&self, path: &str) -> color_eyre::Result<Vec<u8>> {
        use color_eyre::eyre::WrapErr as _;

        let data = self.db.conn()
            .query_row(
                "SELECT fd.data FROM file_data fd JOIN inodes i ON i.id = fd.inode_id WHERE i.path = ?",
                [path],
                |row| row.get(0),
            )
            .wrap_err_with(|| format!("failed to read file data at {path:?}"))?;

        Ok(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_test_overlay() -> color_eyre::Result<(OverlayFs, tempfile::TempDir)> {
        let temp_dir = tempfile::tempdir()?;
        let db_path = temp_dir.path().join("test.loaf");
        let base_path = temp_dir.path().join("base");
        std::fs::create_dir(&base_path)?;

        let overlay = OverlayFs::new(&db_path, &base_path)?;
        Ok((overlay, temp_dir))
    }

    #[test]
    fn test_mkdir() -> color_eyre::Result<()> {
        let (mut overlay, _temp) = setup_test_overlay()?;

        let root_id = overlay.root_id();
        let dir_id = overlay.mkdir(root_id, "testdir", 0o755)?;

        let attrs = overlay.getattr(dir_id)?;
        assert_eq!(attrs.item_type, ItemType::Directory);
        assert_eq!(attrs.mode, 0o755);

        Ok(())
    }

    #[test]
    fn test_symlink() -> color_eyre::Result<()> {
        let (mut overlay, _temp) = setup_test_overlay()?;

        let root_id = overlay.root_id();
        let link_id = overlay.symlink(root_id, "link", "/target/path")?;

        let attrs = overlay.getattr(link_id)?;
        assert_eq!(attrs.item_type, ItemType::Symlink);

        let target = overlay.readlink(link_id)?;
        assert_eq!(target, "/target/path");

        Ok(())
    }

    #[test]
    fn test_rename_file() -> color_eyre::Result<()> {
        let (mut overlay, _temp) = setup_test_overlay()?;

        let root_id = overlay.root_id();
        let file_id = overlay.create(root_id, "old.txt", ItemType::File, 0o644)?;
        overlay.write(file_id, 0, b"test data")?;

        overlay.rename(root_id, "old.txt", root_id, "new.txt")?;

        assert!(overlay.lookup(root_id, "new.txt").is_ok());
        assert!(overlay.lookup(root_id, "old.txt").is_err());

        let new_id = overlay.lookup(root_id, "new.txt")?;
        let mut buf = vec![0u8; 9];
        let n = overlay.read(new_id, 0, &mut buf)?;
        assert_eq!(n, 9);
        assert_eq!(&buf, b"test data");

        Ok(())
    }

    #[test]
    fn test_rename_directory() -> color_eyre::Result<()> {
        let (mut overlay, _temp) = setup_test_overlay()?;

        let root_id = overlay.root_id();
        let dir_id = overlay.mkdir(root_id, "olddir", 0o755)?;
        let _file_id = overlay.create(dir_id, "file.txt", ItemType::File, 0o644)?;

        overlay.rename(root_id, "olddir", root_id, "newdir")?;

        assert!(overlay.lookup(root_id, "newdir").is_ok());
        assert!(overlay.lookup(root_id, "olddir").is_err());

        let new_dir_id = overlay.lookup(root_id, "newdir")?;
        assert!(overlay.lookup(new_dir_id, "file.txt").is_ok());

        Ok(())
    }

    #[test]
    fn test_setattr_mode() -> color_eyre::Result<()> {
        let (mut overlay, _temp) = setup_test_overlay()?;

        let root_id = overlay.root_id();
        let file_id = overlay.create(root_id, "file.txt", ItemType::File, 0o644)?;

        overlay.setattr(file_id, Some(0o600), None, None, None)?;

        let attrs = overlay.getattr(file_id)?;
        assert_eq!(attrs.mode, 0o600);

        Ok(())
    }

    #[test]
    fn test_setattr_size() -> color_eyre::Result<()> {
        let (mut overlay, _temp) = setup_test_overlay()?;

        let root_id = overlay.root_id();
        let file_id = overlay.create(root_id, "file.txt", ItemType::File, 0o644)?;
        overlay.write(file_id, 0, b"hello world")?;

        overlay.setattr(file_id, None, Some(5), None, None)?;

        let attrs = overlay.getattr(file_id)?;
        assert_eq!(attrs.size, 5);

        let mut buf = vec![0u8; 10];
        let n = overlay.read(file_id, 0, &mut buf)?;
        assert_eq!(n, 5);
        assert_eq!(&buf[..5], b"hello");

        Ok(())
    }

    #[test]
    fn test_setattr_times() -> color_eyre::Result<()> {
        let (mut overlay, _temp) = setup_test_overlay()?;

        let root_id = overlay.root_id();
        let file_id = overlay.create(root_id, "file.txt", ItemType::File, 0o644)?;

        let atime = (1000i64, 2000i64);
        let mtime = (3000i64, 4000i64);
        overlay.setattr(file_id, None, None, Some(atime), Some(mtime))?;

        let attrs = overlay.getattr(file_id)?;
        assert_eq!(attrs.atime_sec, 1000);
        assert_eq!(attrs.atime_nsec, 2000);
        assert_eq!(attrs.mtime_sec, 3000);
        assert_eq!(attrs.mtime_nsec, 4000);

        Ok(())
    }

    #[test]
    fn test_readdir_overlay_entries() -> color_eyre::Result<()> {
        let (mut overlay, _temp) = setup_test_overlay()?;

        let root_id = overlay.root_id();
        overlay.create(root_id, "file1.txt", ItemType::File, 0o644)?;
        overlay.mkdir(root_id, "dir1", 0o755)?;
        overlay.symlink(root_id, "link1", "/target")?;

        let entries = overlay.readdir(root_id)?;
        assert_eq!(entries.len(), 3);

        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"file1.txt"));
        assert!(names.contains(&"dir1"));
        assert!(names.contains(&"link1"));

        Ok(())
    }
}
