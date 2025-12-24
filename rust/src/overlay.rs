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
        use color_eyre::eyre::WrapErr as _;

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

    pub fn readdir(&mut self, dir_inode: u64) -> color_eyre::Result<Vec<DirEntry>> {
        let dir_path = self
            .get_path(dir_inode)
            .ok_or_else(|| color_eyre::eyre::eyre!("inode {dir_inode} not found"))?
            .to_string();

        let mut entries = Vec::new();
        let mut seen = std::collections::HashSet::new();

        // TODO: read from overlay DB

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
}
