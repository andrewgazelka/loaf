use std::path::Path;

const SCHEMA_SQL: &str = r#"
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS overlay_config (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS inodes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    parent_id INTEGER NOT NULL,
    name TEXT NOT NULL,
    path TEXT UNIQUE,
    type INTEGER NOT NULL,
    mode INTEGER NOT NULL DEFAULT 420,
    uid INTEGER NOT NULL DEFAULT 0,
    gid INTEGER NOT NULL DEFAULT 0,
    size INTEGER NOT NULL DEFAULT 0,
    link_count INTEGER NOT NULL DEFAULT 1,
    flags INTEGER NOT NULL DEFAULT 0,
    atime_sec INTEGER NOT NULL,
    atime_nsec INTEGER NOT NULL,
    mtime_sec INTEGER NOT NULL,
    mtime_nsec INTEGER NOT NULL,
    ctime_sec INTEGER NOT NULL,
    ctime_nsec INTEGER NOT NULL,
    btime_sec INTEGER NOT NULL,
    btime_nsec INTEGER NOT NULL,
    symlink_target TEXT,
    UNIQUE(parent_id, name)
);

CREATE TABLE IF NOT EXISTS file_data (
    inode_id INTEGER PRIMARY KEY,
    data BLOB NOT NULL DEFAULT X'',
    FOREIGN KEY (inode_id) REFERENCES inodes(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS xattrs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    inode_id INTEGER NOT NULL,
    name TEXT NOT NULL,
    value BLOB NOT NULL,
    UNIQUE(inode_id, name),
    FOREIGN KEY (inode_id) REFERENCES inodes(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS whiteouts (
    path TEXT PRIMARY KEY
);

CREATE INDEX IF NOT EXISTS idx_inodes_parent ON inodes(parent_id);
CREATE INDEX IF NOT EXISTS idx_inodes_path ON inodes(path);
CREATE INDEX IF NOT EXISTS idx_xattrs_inode ON xattrs(inode_id);
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ItemType {
    File = 0,
    Directory = 1,
    Symlink = 2,
}

impl TryFrom<i64> for ItemType {
    type Error = color_eyre::Report;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::File),
            1 => Ok(Self::Directory),
            2 => Ok(Self::Symlink),
            _ => color_eyre::eyre::bail!("invalid item type: {value}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Attrs {
    pub file_id: u64,
    pub parent_id: u64,
    pub item_type: ItemType,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub size: u64,
    pub link_count: u32,
    pub atime_sec: i64,
    pub atime_nsec: i64,
    pub mtime_sec: i64,
    pub mtime_nsec: i64,
    pub ctime_sec: i64,
    pub ctime_nsec: i64,
}

#[derive(Debug, Clone)]
pub struct DirEntry {
    pub inode_id: u64,
    pub name: String,
    pub item_type: ItemType,
}

fn now_timespec() -> (i64, i64) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    (now.as_secs() as i64, now.subsec_nanos() as i64)
}

pub struct Database {
    conn: rusqlite::Connection,
}

impl Database {
    pub fn open(path: &Path) -> color_eyre::Result<Self> {
        use color_eyre::eyre::WrapErr as _;

        let conn = rusqlite::Connection::open(path)
            .wrap_err_with(|| format!("failed to open database at {path:?}"))?;

        conn.execute_batch(SCHEMA_SQL)
            .wrap_err("failed to initialize schema")?;

        let (sec, nsec) = now_timespec();
        conn.execute(
            "INSERT OR IGNORE INTO inodes (id, parent_id, name, type, mode, uid, gid, size, link_count,
                atime_sec, atime_nsec, mtime_sec, mtime_nsec, ctime_sec, ctime_nsec, btime_sec, btime_nsec)
            VALUES (1, 0, '/', 1, 448, 0, 0, 0, 1, ?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![sec, nsec, sec, nsec, sec, nsec, sec, nsec],
        )
        .wrap_err("failed to insert root inode")?;

        Ok(Self { conn })
    }

    pub fn set_base_path(&self, base_path: &str) -> color_eyre::Result<()> {
        use color_eyre::eyre::WrapErr as _;

        self.conn
            .execute(
                "INSERT OR REPLACE INTO overlay_config (key, value) VALUES ('base_path', ?)",
                [base_path],
            )
            .wrap_err("failed to set base path")?;
        Ok(())
    }

    pub fn get_base_path(&self) -> color_eyre::Result<String> {
        use color_eyre::eyre::WrapErr as _;

        self.conn
            .query_row(
                "SELECT value FROM overlay_config WHERE key = 'base_path'",
                [],
                |row| row.get(0),
            )
            .wrap_err("failed to get base path")
    }

    pub fn exists_by_path(&self, path: &str) -> bool {
        self.conn
            .query_row(
                "SELECT 1 FROM inodes WHERE path = ?",
                [path],
                |_| Ok(()),
            )
            .is_ok()
    }

    pub fn get_attrs_by_path(&self, path: &str) -> color_eyre::Result<Attrs> {
        use color_eyre::eyre::WrapErr as _;

        self.conn
            .query_row(
                "SELECT id, parent_id, type, mode, uid, gid, size, link_count,
                        atime_sec, atime_nsec, mtime_sec, mtime_nsec, ctime_sec, ctime_nsec
                 FROM inodes WHERE path = ?",
                [path],
                |row| {
                    Ok(Attrs {
                        file_id: row.get::<_, i64>(0)? as u64,
                        parent_id: row.get::<_, i64>(1)? as u64,
                        item_type: ItemType::try_from(row.get::<_, i64>(2)?).unwrap(),
                        mode: row.get(3)?,
                        uid: row.get(4)?,
                        gid: row.get(5)?,
                        size: row.get::<_, i64>(6)? as u64,
                        link_count: row.get(7)?,
                        atime_sec: row.get(8)?,
                        atime_nsec: row.get(9)?,
                        mtime_sec: row.get(10)?,
                        mtime_nsec: row.get(11)?,
                        ctime_sec: row.get(12)?,
                        ctime_nsec: row.get(13)?,
                    })
                },
            )
            .wrap_err_with(|| format!("failed to get attrs for path {path:?}"))
    }

    pub fn create_by_path(&self, path: &str, item_type: ItemType, mode: u32) -> color_eyre::Result<u64> {
        use color_eyre::eyre::WrapErr as _;

        let (sec, nsec) = now_timespec();
        let name = std::path::Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");

        self.conn
            .execute(
                "INSERT INTO inodes (parent_id, name, path, type, mode, atime_sec, atime_nsec,
                    mtime_sec, mtime_nsec, ctime_sec, ctime_nsec, btime_sec, btime_nsec)
                VALUES (0, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![name, path, item_type as u8, mode, sec, nsec, sec, nsec, sec, nsec, sec, nsec],
            )
            .wrap_err_with(|| format!("failed to create inode at {path:?}"))?;

        let inode_id = self.conn.last_insert_rowid() as u64;

        if item_type == ItemType::File {
            self.conn
                .execute(
                    "INSERT INTO file_data (inode_id, data) VALUES (?, X'')",
                    [inode_id as i64],
                )
                .wrap_err("failed to create file_data entry")?;
        }

        Ok(inode_id)
    }

    pub fn create_symlink_by_path(&self, path: &str, target: &str) -> color_eyre::Result<u64> {
        use color_eyre::eyre::WrapErr as _;

        let (sec, nsec) = now_timespec();
        let name = std::path::Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");

        self.conn
            .execute(
                "INSERT INTO inodes (parent_id, name, path, type, mode, symlink_target,
                    atime_sec, atime_nsec, mtime_sec, mtime_nsec, ctime_sec, ctime_nsec, btime_sec, btime_nsec)
                VALUES (0, ?, ?, 2, 511, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![name, path, target, sec, nsec, sec, nsec, sec, nsec, sec, nsec],
            )
            .wrap_err_with(|| format!("failed to create symlink at {path:?}"))?;

        Ok(self.conn.last_insert_rowid() as u64)
    }

    pub fn remove_by_path(&self, path: &str) -> color_eyre::Result<()> {
        use color_eyre::eyre::WrapErr as _;

        self.conn
            .execute("DELETE FROM inodes WHERE path = ?", [path])
            .wrap_err_with(|| format!("failed to remove inode at {path:?}"))?;
        Ok(())
    }

    pub fn read_by_path(&self, path: &str, offset: u64, buf: &mut [u8]) -> color_eyre::Result<usize> {
        use color_eyre::eyre::WrapErr as _;

        let data: Vec<u8> = self
            .conn
            .query_row(
                "SELECT fd.data FROM file_data fd JOIN inodes i ON i.id = fd.inode_id WHERE i.path = ?",
                [path],
                |row| row.get(0),
            )
            .wrap_err_with(|| format!("failed to read file at {path:?}"))?;

        let offset = offset as usize;
        if offset >= data.len() {
            return Ok(0);
        }

        let available = data.len() - offset;
        let to_copy = available.min(buf.len());
        buf[..to_copy].copy_from_slice(&data[offset..offset + to_copy]);
        Ok(to_copy)
    }

    pub fn write_by_path(&self, path: &str, offset: u64, data: &[u8]) -> color_eyre::Result<usize> {
        use color_eyre::eyre::WrapErr as _;

        let inode_id: i64 = self
            .conn
            .query_row("SELECT id FROM inodes WHERE path = ?", [path], |row| row.get(0))
            .wrap_err_with(|| format!("failed to find inode for {path:?}"))?;

        let current: Vec<u8> = self
            .conn
            .query_row(
                "SELECT data FROM file_data WHERE inode_id = ?",
                [inode_id],
                |row| row.get(0),
            )
            .unwrap_or_default();

        let offset = offset as usize;
        let new_size = (offset + data.len()).max(current.len());
        let mut new_data = vec![0u8; new_size];

        if !current.is_empty() {
            new_data[..current.len()].copy_from_slice(&current);
        }
        new_data[offset..offset + data.len()].copy_from_slice(data);

        let (sec, nsec) = now_timespec();

        self.conn
            .execute(
                "UPDATE file_data SET data = ? WHERE inode_id = ?",
                rusqlite::params![new_data, inode_id],
            )
            .wrap_err("failed to update file data")?;

        self.conn
            .execute(
                "UPDATE inodes SET size = ?, mtime_sec = ?, mtime_nsec = ? WHERE id = ?",
                rusqlite::params![new_size as i64, sec, nsec, inode_id],
            )
            .wrap_err("failed to update inode size")?;

        Ok(data.len())
    }

    pub fn read_symlink_by_path(&self, path: &str) -> color_eyre::Result<String> {
        use color_eyre::eyre::WrapErr as _;

        self.conn
            .query_row(
                "SELECT symlink_target FROM inodes WHERE path = ?",
                [path],
                |row| row.get(0),
            )
            .wrap_err_with(|| format!("failed to read symlink at {path:?}"))
    }

    pub fn add_whiteout(&self, path: &str) -> color_eyre::Result<()> {
        use color_eyre::eyre::WrapErr as _;

        self.conn
            .execute("INSERT OR IGNORE INTO whiteouts (path) VALUES (?)", [path])
            .wrap_err_with(|| format!("failed to add whiteout for {path:?}"))?;
        Ok(())
    }

    pub fn remove_whiteout(&self, path: &str) -> color_eyre::Result<()> {
        self.conn
            .execute("DELETE FROM whiteouts WHERE path = ?", [path])
            .ok();
        Ok(())
    }

    pub fn is_whiteout(&self, path: &str) -> bool {
        self.conn
            .query_row("SELECT 1 FROM whiteouts WHERE path = ?", [path], |_| Ok(()))
            .is_ok()
    }

    /// List all children of a directory by parent path
    pub fn list_children_by_parent_path(&self, parent_path: &str) -> color_eyre::Result<Vec<DirEntry>> {
        use color_eyre::eyre::WrapErr as _;

        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, name, type FROM inodes
                 WHERE path LIKE ? || '/%' AND path NOT LIKE ? || '/%/%'",
            )
            .wrap_err("failed to prepare list_children query")?;

        let entries = stmt
            .query_map([parent_path, parent_path], |row| {
                Ok(DirEntry {
                    inode_id: row.get::<_, i64>(0)? as u64,
                    name: row.get(1)?,
                    item_type: ItemType::try_from(row.get::<_, i64>(2)?).unwrap(),
                })
            })
            .wrap_err_with(|| format!("failed to query children of {parent_path:?}"))?
            .collect::<Result<Vec<_>, _>>()
            .wrap_err("failed to collect directory entries")?;

        Ok(entries)
    }

    /// Rename (move) a file or directory from one path to another
    pub fn rename_by_path(&self, old_path: &str, new_path: &str) -> color_eyre::Result<()> {
        use color_eyre::eyre::WrapErr as _;

        let (sec, nsec) = now_timespec();
        let new_name = std::path::Path::new(new_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");

        // For directories, also update all descendant paths
        let attrs = self.get_attrs_by_path(old_path)?;

        if attrs.item_type == ItemType::Directory {
            // Update all paths that start with old_path/
            let old_prefix = format!("{}/", old_path);
            let new_prefix = format!("{}/", new_path);

            self.conn
                .execute(
                    "UPDATE inodes SET path = ? || SUBSTR(path, ?)
                     WHERE path LIKE ? || '%'",
                    rusqlite::params![new_prefix, old_prefix.len() + 1, old_prefix],
                )
                .wrap_err("failed to update descendant paths")?;
        }

        // Update the target inode itself
        self.conn
            .execute(
                "UPDATE inodes SET path = ?, name = ?, ctime_sec = ?, ctime_nsec = ?
                 WHERE path = ?",
                rusqlite::params![new_path, new_name, sec, nsec, old_path],
            )
            .wrap_err_with(|| format!("failed to rename {old_path:?} to {new_path:?}"))?;

        Ok(())
    }

    /// Truncate file to specified size
    pub fn truncate_by_path(&self, path: &str, size: u64) -> color_eyre::Result<()> {
        use color_eyre::eyre::WrapErr as _;

        let inode_id: i64 = self
            .conn
            .query_row("SELECT id FROM inodes WHERE path = ?", [path], |row| row.get(0))
            .wrap_err_with(|| format!("failed to find inode for {path:?}"))?;

        let current: Vec<u8> = self
            .conn
            .query_row(
                "SELECT data FROM file_data WHERE inode_id = ?",
                [inode_id],
                |row| row.get(0),
            )
            .unwrap_or_default();

        let size = size as usize;
        let new_data = if size >= current.len() {
            let mut data = current;
            data.resize(size, 0);
            data
        } else {
            current[..size].to_vec()
        };

        let (sec, nsec) = now_timespec();

        self.conn
            .execute(
                "UPDATE file_data SET data = ? WHERE inode_id = ?",
                rusqlite::params![new_data, inode_id],
            )
            .wrap_err("failed to update file data")?;

        self.conn
            .execute(
                "UPDATE inodes SET size = ?, mtime_sec = ?, mtime_nsec = ? WHERE id = ?",
                rusqlite::params![size as i64, sec, nsec, inode_id],
            )
            .wrap_err("failed to update inode size")?;

        Ok(())
    }

    /// Update access and modification times
    pub fn update_times_by_path(
        &self,
        path: &str,
        atime_sec: Option<i64>,
        atime_nsec: Option<i64>,
        mtime_sec: Option<i64>,
        mtime_nsec: Option<i64>,
    ) -> color_eyre::Result<()> {
        use color_eyre::eyre::WrapErr as _;

        let (default_sec, default_nsec) = now_timespec();

        self.conn
            .execute(
                "UPDATE inodes SET atime_sec = ?, atime_nsec = ?, mtime_sec = ?, mtime_nsec = ?
                 WHERE path = ?",
                rusqlite::params![
                    atime_sec.unwrap_or(default_sec),
                    atime_nsec.unwrap_or(default_nsec),
                    mtime_sec.unwrap_or(default_sec),
                    mtime_nsec.unwrap_or(default_nsec),
                    path,
                ],
            )
            .wrap_err_with(|| format!("failed to update times for {path:?}"))?;

        Ok(())
    }

    /// Update mode (permissions) for a path
    pub fn update_mode_by_path(&self, path: &str, mode: u32) -> color_eyre::Result<()> {
        use color_eyre::eyre::WrapErr as _;

        let (sec, nsec) = now_timespec();

        self.conn
            .execute(
                "UPDATE inodes SET mode = ?, ctime_sec = ?, ctime_nsec = ? WHERE path = ?",
                rusqlite::params![mode, sec, nsec, path],
            )
            .wrap_err_with(|| format!("failed to update mode for {path:?}"))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_test_db() -> color_eyre::Result<Database> {
        let db = Database::open(std::path::Path::new(":memory:"))?;
        db.set_base_path("/test")?;
        Ok(db)
    }

    #[test]
    fn test_list_children_empty() -> color_eyre::Result<()> {
        let db = setup_test_db()?;
        let children = db.list_children_by_parent_path("/")?;
        assert_eq!(children.len(), 0);
        Ok(())
    }

    #[test]
    fn test_list_children_with_files() -> color_eyre::Result<()> {
        let db = setup_test_db()?;

        db.create_by_path("/file1.txt", ItemType::File, 0o644)?;
        db.create_by_path("/file2.txt", ItemType::File, 0o644)?;
        db.create_by_path("/dir", ItemType::Directory, 0o755)?;

        let children = db.list_children_by_parent_path("/")?;
        assert_eq!(children.len(), 3);

        let names: Vec<_> = children.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"file1.txt"));
        assert!(names.contains(&"file2.txt"));
        assert!(names.contains(&"dir"));

        Ok(())
    }

    #[test]
    fn test_list_children_nested() -> color_eyre::Result<()> {
        let db = setup_test_db()?;

        db.create_by_path("/dir", ItemType::Directory, 0o755)?;
        db.create_by_path("/dir/file.txt", ItemType::File, 0o644)?;
        db.create_by_path("/dir/subdir", ItemType::Directory, 0o755)?;
        db.create_by_path("/dir/subdir/nested.txt", ItemType::File, 0o644)?;

        // List immediate children of /dir
        let children = db.list_children_by_parent_path("/dir")?;
        assert_eq!(children.len(), 2);

        let names: Vec<_> = children.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"file.txt"));
        assert!(names.contains(&"subdir"));
        assert!(!names.contains(&"nested.txt")); // Not a direct child

        Ok(())
    }

    #[test]
    fn test_rename_file() -> color_eyre::Result<()> {
        let db = setup_test_db()?;

        let inode_id = db.create_by_path("/old.txt", ItemType::File, 0o644)?;
        db.rename_by_path("/old.txt", "/new.txt")?;

        // Old path should not exist
        assert!(!db.exists_by_path("/old.txt"));

        // New path should exist with same inode
        let attrs = db.get_attrs_by_path("/new.txt")?;
        assert_eq!(attrs.file_id, inode_id);

        Ok(())
    }

    #[test]
    fn test_rename_directory() -> color_eyre::Result<()> {
        let db = setup_test_db()?;

        db.create_by_path("/olddir", ItemType::Directory, 0o755)?;
        db.create_by_path("/olddir/file.txt", ItemType::File, 0o644)?;
        db.create_by_path("/olddir/subdir", ItemType::Directory, 0o755)?;
        db.create_by_path("/olddir/subdir/nested.txt", ItemType::File, 0o644)?;

        db.rename_by_path("/olddir", "/newdir")?;

        // Old paths should not exist
        assert!(!db.exists_by_path("/olddir"));
        assert!(!db.exists_by_path("/olddir/file.txt"));

        // New paths should exist
        assert!(db.exists_by_path("/newdir"));
        assert!(db.exists_by_path("/newdir/file.txt"));
        assert!(db.exists_by_path("/newdir/subdir"));
        assert!(db.exists_by_path("/newdir/subdir/nested.txt"));

        Ok(())
    }

    #[test]
    fn test_truncate_expand() -> color_eyre::Result<()> {
        let db = setup_test_db()?;

        db.create_by_path("/file.txt", ItemType::File, 0o644)?;
        db.write_by_path("/file.txt", 0, b"hello")?;

        db.truncate_by_path("/file.txt", 10)?;

        let attrs = db.get_attrs_by_path("/file.txt")?;
        assert_eq!(attrs.size, 10);

        let mut buf = vec![0u8; 10];
        db.read_by_path("/file.txt", 0, &mut buf)?;
        assert_eq!(&buf[..5], b"hello");
        assert_eq!(&buf[5..], &[0, 0, 0, 0, 0]);

        Ok(())
    }

    #[test]
    fn test_truncate_shrink() -> color_eyre::Result<()> {
        let db = setup_test_db()?;

        db.create_by_path("/file.txt", ItemType::File, 0o644)?;
        db.write_by_path("/file.txt", 0, b"hello world")?;

        db.truncate_by_path("/file.txt", 5)?;

        let attrs = db.get_attrs_by_path("/file.txt")?;
        assert_eq!(attrs.size, 5);

        let mut buf = vec![0u8; 10];
        let n = db.read_by_path("/file.txt", 0, &mut buf)?;
        assert_eq!(n, 5);
        assert_eq!(&buf[..5], b"hello");

        Ok(())
    }

    #[test]
    fn test_update_times() -> color_eyre::Result<()> {
        let db = setup_test_db()?;

        db.create_by_path("/file.txt", ItemType::File, 0o644)?;

        let atime_sec = 1000i64;
        let atime_nsec = 2000i64;
        let mtime_sec = 3000i64;
        let mtime_nsec = 4000i64;

        db.update_times_by_path(
            "/file.txt",
            Some(atime_sec),
            Some(atime_nsec),
            Some(mtime_sec),
            Some(mtime_nsec),
        )?;

        let attrs = db.get_attrs_by_path("/file.txt")?;
        assert_eq!(attrs.atime_sec, atime_sec);
        assert_eq!(attrs.atime_nsec, atime_nsec);
        assert_eq!(attrs.mtime_sec, mtime_sec);
        assert_eq!(attrs.mtime_nsec, mtime_nsec);

        Ok(())
    }
}
