const std = @import("std");
const sqlite = @import("sqlite");
const err = @import("error.zig");

pub const Error = err.Error;

pub const ItemType = enum(u8) {
    file = 0,
    directory = 1,
    symlink = 2,
};

pub const Timespec = struct {
    sec: i64,
    nsec: i64,

    pub fn now() Timespec {
        const ts = std.time.nanoTimestamp();
        const sec: i64 = @intCast(@divFloor(ts, std.time.ns_per_s));
        const nsec: i64 = @intCast(@mod(ts, std.time.ns_per_s));
        return .{ .sec = sec, .nsec = nsec };
    }
};

pub const Attrs = extern struct {
    file_id: u64,
    parent_id: u64,
    item_type: u8,
    _pad1: [3]u8 = .{ 0, 0, 0 },
    mode: u32,
    uid: u32,
    gid: u32,
    size: u64,
    alloc_size: u64,
    link_count: u32,
    flags: u32,
    atime_sec: i64,
    atime_nsec: i64,
    mtime_sec: i64,
    mtime_nsec: i64,
    ctime_sec: i64,
    ctime_nsec: i64,
    btime_sec: i64,
    btime_nsec: i64,
};

pub const DirEntry = struct {
    inode_id: u64,
    name: []const u8,
    item_type: ItemType,
};

const schema_sql =
    \\PRAGMA foreign_keys = ON;
    \\
    \\-- Overlay configuration
    \\CREATE TABLE IF NOT EXISTS overlay_config (
    \\    key TEXT PRIMARY KEY,
    \\    value TEXT NOT NULL
    \\);
    \\
    \\-- Files/directories in the overlay (path-indexed for overlay mode)
    \\CREATE TABLE IF NOT EXISTS inodes (
    \\    id INTEGER PRIMARY KEY AUTOINCREMENT,
    \\    parent_id INTEGER NOT NULL,
    \\    name TEXT NOT NULL,
    \\    path TEXT UNIQUE,
    \\    type INTEGER NOT NULL,
    \\    mode INTEGER NOT NULL DEFAULT 420,
    \\    uid INTEGER NOT NULL DEFAULT 0,
    \\    gid INTEGER NOT NULL DEFAULT 0,
    \\    size INTEGER NOT NULL DEFAULT 0,
    \\    link_count INTEGER NOT NULL DEFAULT 1,
    \\    flags INTEGER NOT NULL DEFAULT 0,
    \\    atime_sec INTEGER NOT NULL,
    \\    atime_nsec INTEGER NOT NULL,
    \\    mtime_sec INTEGER NOT NULL,
    \\    mtime_nsec INTEGER NOT NULL,
    \\    ctime_sec INTEGER NOT NULL,
    \\    ctime_nsec INTEGER NOT NULL,
    \\    btime_sec INTEGER NOT NULL,
    \\    btime_nsec INTEGER NOT NULL,
    \\    symlink_target TEXT,
    \\    UNIQUE(parent_id, name)
    \\);
    \\
    \\-- File content blobs
    \\CREATE TABLE IF NOT EXISTS file_data (
    \\    inode_id INTEGER PRIMARY KEY,
    \\    data BLOB NOT NULL DEFAULT X'',
    \\    FOREIGN KEY (inode_id) REFERENCES inodes(id) ON DELETE CASCADE
    \\);
    \\
    \\-- Extended attributes
    \\CREATE TABLE IF NOT EXISTS xattrs (
    \\    id INTEGER PRIMARY KEY AUTOINCREMENT,
    \\    inode_id INTEGER NOT NULL,
    \\    name TEXT NOT NULL,
    \\    value BLOB NOT NULL,
    \\    UNIQUE(inode_id, name),
    \\    FOREIGN KEY (inode_id) REFERENCES inodes(id) ON DELETE CASCADE
    \\);
    \\
    \\-- Whiteouts: paths deleted in overlay (hides real fs entries)
    \\CREATE TABLE IF NOT EXISTS whiteouts (
    \\    path TEXT PRIMARY KEY
    \\);
    \\
    \\CREATE INDEX IF NOT EXISTS idx_inodes_parent ON inodes(parent_id);
    \\CREATE INDEX IF NOT EXISTS idx_inodes_path ON inodes(path);
    \\CREATE INDEX IF NOT EXISTS idx_xattrs_inode ON xattrs(inode_id);
;

const root_insert_sql =
    \\INSERT OR IGNORE INTO inodes (id, parent_id, name, type, mode, uid, gid, size, link_count,
    \\    atime_sec, atime_nsec, mtime_sec, mtime_nsec, ctime_sec, ctime_nsec, btime_sec, btime_nsec)
    \\VALUES (1, 0, '/', 1, 448, 0, 0, 0, 1, ?, ?, ?, ?, ?, ?, ?, ?);
;

pub const Database = struct {
    db: sqlite.Db,
    allocator: std.mem.Allocator,

    pub const InitOptions = struct {
        path: [:0]const u8,
        allocator: std.mem.Allocator,
    };

    pub fn init(options: InitOptions) Error!Database {
        var db = sqlite.Db.init(.{
            .mode = .{ .File = options.path },
            .open_flags = .{ .write = true, .create = true },
            .threading_mode = .Serialized,
        }) catch return error.IoError;

        // Initialize schema
        db.execMulti(schema_sql, .{}) catch {
            db.deinit();
            return error.SchemaInitFailed;
        };

        // Insert root inode if not exists
        const now_ts = Timespec.now();
        db.exec(root_insert_sql, .{}, .{
            now_ts.sec, now_ts.nsec, now_ts.sec, now_ts.nsec,
            now_ts.sec, now_ts.nsec, now_ts.sec, now_ts.nsec,
        }) catch {
            db.deinit();
            return error.RootInsertFailed;
        };

        return .{ .db = db, .allocator = options.allocator };
    }

    pub fn deinit(self: *Database) void {
        self.db.deinit();
    }

    pub fn getAttrs(self: *Database, inode_id: u64) Error!Attrs {
        const query =
            \\SELECT id, parent_id, type, mode, uid, gid, size, link_count, flags,
            \\       atime_sec, atime_nsec, mtime_sec, mtime_nsec,
            \\       ctime_sec, ctime_nsec, btime_sec, btime_nsec
            \\FROM inodes WHERE id = ?
        ;

        const Row = struct {
            id: u64,
            parent_id: u64,
            type: u8,
            mode: u32,
            uid: u32,
            gid: u32,
            size: u64,
            link_count: u32,
            flags: u32,
            atime_sec: i64,
            atime_nsec: i64,
            mtime_sec: i64,
            mtime_nsec: i64,
            ctime_sec: i64,
            ctime_nsec: i64,
            btime_sec: i64,
            btime_nsec: i64,
        };

        const row = self.db.one(Row, query, .{}, .{inode_id}) catch {
            return error.QueryFailed;
        } orelse return error.NotFound;

        return Attrs{
            .file_id = row.id,
            .parent_id = row.parent_id,
            .item_type = row.type,
            .mode = row.mode,
            .uid = row.uid,
            .gid = row.gid,
            .size = row.size,
            .alloc_size = row.size,
            .link_count = row.link_count,
            .flags = row.flags,
            .atime_sec = row.atime_sec,
            .atime_nsec = row.atime_nsec,
            .mtime_sec = row.mtime_sec,
            .mtime_nsec = row.mtime_nsec,
            .ctime_sec = row.ctime_sec,
            .ctime_nsec = row.ctime_nsec,
            .btime_sec = row.btime_sec,
            .btime_nsec = row.btime_nsec,
        };
    }

    pub fn lookup(self: *Database, parent_id: u64, name: []const u8) Error!u64 {
        const query = "SELECT id FROM inodes WHERE parent_id = ? AND name = ?";
        const row = self.db.one(struct { id: u64 }, query, .{}, .{ parent_id, name }) catch {
            return error.QueryFailed;
        } orelse return error.NotFound;
        return row.id;
    }

    pub fn create(self: *Database, parent_id: u64, name: []const u8, item_type: ItemType, mode: u32) Error!u64 {
        const now_ts = Timespec.now();
        const query =
            \\INSERT INTO inodes (parent_id, name, type, mode, atime_sec, atime_nsec,
            \\    mtime_sec, mtime_nsec, ctime_sec, ctime_nsec, btime_sec, btime_nsec)
            \\VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ;

        self.db.exec(query, .{}, .{
            parent_id,  name,        @intFromEnum(item_type), mode,
            now_ts.sec, now_ts.nsec, now_ts.sec,              now_ts.nsec,
            now_ts.sec, now_ts.nsec, now_ts.sec,              now_ts.nsec,
        }) catch {
            return error.InsertFailed;
        };

        const inode_id: u64 = @intCast(self.db.getLastInsertRowID());

        // Create file_data entry for files
        if (item_type == .file) {
            self.db.exec("INSERT INTO file_data (inode_id, data) VALUES (?, X'')", .{}, .{inode_id}) catch {
                return error.InsertFailed;
            };
        }

        return inode_id;
    }

    pub fn createSymlink(self: *Database, parent_id: u64, name: []const u8, target: []const u8) Error!u64 {
        const now_ts = Timespec.now();
        const query =
            \\INSERT INTO inodes (parent_id, name, type, mode, symlink_target,
            \\    atime_sec, atime_nsec, mtime_sec, mtime_nsec, ctime_sec, ctime_nsec, btime_sec, btime_nsec)
            \\VALUES (?, ?, 2, 511, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ;

        self.db.exec(query, .{}, .{
            parent_id,   name,        target,
            now_ts.sec,  now_ts.nsec, now_ts.sec,
            now_ts.nsec, now_ts.sec,  now_ts.nsec,
            now_ts.sec,  now_ts.nsec,
        }) catch {
            return error.InsertFailed;
        };

        return @intCast(self.db.getLastInsertRowID());
    }

    pub fn remove(self: *Database, inode_id: u64) Error!void {
        self.db.exec("DELETE FROM inodes WHERE id = ?", .{}, .{inode_id}) catch {
            return error.DeleteFailed;
        };
    }

    pub fn rename(self: *Database, inode_id: u64, new_parent_id: u64, new_name: []const u8) Error!void {
        const now_ts = Timespec.now();
        self.db.exec(
            "UPDATE inodes SET parent_id = ?, name = ?, ctime_sec = ?, ctime_nsec = ? WHERE id = ?",
            .{},
            .{ new_parent_id, new_name, now_ts.sec, now_ts.nsec, inode_id },
        ) catch {
            return error.UpdateFailed;
        };
    }

    pub fn read(self: *Database, inode_id: u64, offset: i64, buf: []u8) Error!usize {
        const query = "SELECT data FROM file_data WHERE inode_id = ?";
        const row = self.db.oneAlloc(struct { data: []const u8 }, self.allocator, query, .{}, .{inode_id}) catch {
            return error.QueryFailed;
        } orelse return error.NotFound;
        defer self.allocator.free(row.data);

        const data = row.data;
        if (offset < 0 or @as(u64, @intCast(offset)) >= data.len) {
            return 0;
        }

        const start: usize = @intCast(offset);
        const available = data.len - start;
        const to_copy = @min(available, buf.len);
        @memcpy(buf[0..to_copy], data[start..][0..to_copy]);
        return to_copy;
    }

    pub fn write(self: *Database, inode_id: u64, offset: i64, data: []const u8) Error!usize {
        if (offset < 0) return error.InvalidOffset;

        // Get current data
        const query = "SELECT data FROM file_data WHERE inode_id = ?";
        const row = self.db.oneAlloc(struct { data: []const u8 }, self.allocator, query, .{}, .{inode_id}) catch {
            return error.QueryFailed;
        } orelse return error.NotFound;
        defer self.allocator.free(row.data);

        const current = row.data;
        const off: usize = @intCast(offset);
        const new_size = @max(current.len, off + data.len);

        // Build new data
        const new_data = self.allocator.alloc(u8, new_size) catch return error.OutOfMemory;
        defer self.allocator.free(new_data);

        @memset(new_data, 0);
        if (current.len > 0) {
            @memcpy(new_data[0..current.len], current);
        }
        @memcpy(new_data[off..][0..data.len], data);

        // Update
        const now_ts = Timespec.now();
        self.db.exec("UPDATE file_data SET data = ? WHERE inode_id = ?", .{}, .{
            sqlite.Blob{ .data = new_data },
            inode_id,
        }) catch {
            return error.UpdateFailed;
        };

        self.db.exec(
            "UPDATE inodes SET size = ?, mtime_sec = ?, mtime_nsec = ? WHERE id = ?",
            .{},
            .{ new_size, now_ts.sec, now_ts.nsec, inode_id },
        ) catch {
            return error.UpdateFailed;
        };

        return data.len;
    }

    pub fn readSymlink(self: *Database, inode_id: u64, buf: []u8) Error!usize {
        const query = "SELECT symlink_target FROM inodes WHERE id = ?";
        const row = self.db.oneAlloc(struct { symlink_target: ?[]const u8 }, self.allocator, query, .{}, .{inode_id}) catch {
            return error.QueryFailed;
        } orelse return error.NotFound;
        defer if (row.symlink_target) |t| self.allocator.free(t);

        const target = row.symlink_target orelse return error.NotASymlink;
        const to_copy = @min(target.len, buf.len);
        @memcpy(buf[0..to_copy], target[0..to_copy]);
        return to_copy;
    }

    pub fn readDir(self: *Database, dir_inode_id: u64, allocator: std.mem.Allocator) Error![]DirEntry {
        const query = "SELECT id, name, type FROM inodes WHERE parent_id = ?";

        var stmt = self.db.prepare(query) catch return error.QueryFailed;
        defer stmt.deinit();

        var entries: std.ArrayList(DirEntry) = .{};
        errdefer {
            for (entries.items) |entry| {
                allocator.free(entry.name);
            }
            entries.deinit(allocator);
        }

        var iter = stmt.iterator(struct { id: u64, name: []const u8, type: u8 }, .{dir_inode_id}) catch return error.QueryFailed;

        while (iter.nextAlloc(allocator, .{}) catch return error.QueryFailed) |row| {
            // nextAlloc already allocates the string, so we own it
            entries.append(allocator, .{
                .inode_id = row.id,
                .name = row.name,
                .item_type = @enumFromInt(row.type),
            }) catch {
                allocator.free(row.name);
                return error.OutOfMemory;
            };
        }

        return entries.toOwnedSlice(allocator) catch return error.OutOfMemory;
    }

    pub fn sync(self: *Database) Error!void {
        // SQLite's default journal mode (DELETE) is already durable - each transaction
        // is synced to disk on commit. This is a no-op but provides an explicit sync point.
        _ = self;
    }

    // =========================================================================
    // Overlay-specific operations (path-based)
    // =========================================================================

    /// Store the base path for this overlay.
    pub fn setBasePath(self: *Database, base_path: []const u8) Error!void {
        self.db.exec(
            "INSERT OR REPLACE INTO overlay_config (key, value) VALUES ('base_path', ?)",
            .{},
            .{base_path},
        ) catch return error.InsertFailed;
    }

    /// Get the stored base path.
    pub fn getBasePath(self: *Database) Error![]const u8 {
        const row = (self.db.oneAlloc(
            struct { value: []const u8 },
            self.allocator,
            "SELECT value FROM overlay_config WHERE key = 'base_path'",
            .{},
            .{},
        ) catch return error.QueryFailed) orelse return error.NotFound;
        return row.value;
    }

    /// Check if a path exists in the overlay.
    pub fn existsByPath(self: *Database, path: []const u8) bool {
        const row = self.db.one(
            struct { id: u64 },
            "SELECT id FROM inodes WHERE path = ?",
            .{},
            .{path},
        ) catch return false;
        return row != null;
    }

    /// Get attrs by path.
    pub fn getAttrsByPath(self: *Database, path: []const u8) Error!Attrs {
        const query =
            \\SELECT id, parent_id, type, mode, uid, gid, size, link_count, flags,
            \\       atime_sec, atime_nsec, mtime_sec, mtime_nsec,
            \\       ctime_sec, ctime_nsec, btime_sec, btime_nsec
            \\FROM inodes WHERE path = ?
        ;

        const Row = struct {
            id: u64,
            parent_id: u64,
            type: u8,
            mode: u32,
            uid: u32,
            gid: u32,
            size: u64,
            link_count: u32,
            flags: u32,
            atime_sec: i64,
            atime_nsec: i64,
            mtime_sec: i64,
            mtime_nsec: i64,
            ctime_sec: i64,
            ctime_nsec: i64,
            btime_sec: i64,
            btime_nsec: i64,
        };

        const row = self.db.one(Row, query, .{}, .{path}) catch {
            return error.QueryFailed;
        } orelse return error.NotFound;

        return Attrs{
            .file_id = row.id,
            .parent_id = row.parent_id,
            .item_type = row.type,
            .mode = row.mode,
            .uid = row.uid,
            .gid = row.gid,
            .size = row.size,
            .alloc_size = row.size,
            .link_count = row.link_count,
            .flags = row.flags,
            .atime_sec = row.atime_sec,
            .atime_nsec = row.atime_nsec,
            .mtime_sec = row.mtime_sec,
            .mtime_nsec = row.mtime_nsec,
            .ctime_sec = row.ctime_sec,
            .ctime_nsec = row.ctime_nsec,
            .btime_sec = row.btime_sec,
            .btime_nsec = row.btime_nsec,
        };
    }

    /// Create an entry by path.
    pub fn createByPath(self: *Database, path: []const u8, item_type: ItemType, mode: u32) Error!u64 {
        const now_ts = Timespec.now();
        const name = std.fs.path.basename(path);

        self.db.exec(
            \\INSERT INTO inodes (parent_id, name, path, type, mode, atime_sec, atime_nsec,
            \\    mtime_sec, mtime_nsec, ctime_sec, ctime_nsec, btime_sec, btime_nsec)
            \\VALUES (0, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ,
            .{},
            .{
                name,       path,        @intFromEnum(item_type), mode,
                now_ts.sec, now_ts.nsec, now_ts.sec,              now_ts.nsec,
                now_ts.sec, now_ts.nsec, now_ts.sec,              now_ts.nsec,
            },
        ) catch return error.InsertFailed;

        const inode_id: u64 = @intCast(self.db.getLastInsertRowID());

        if (item_type == .file) {
            self.db.exec("INSERT INTO file_data (inode_id, data) VALUES (?, X'')", .{}, .{inode_id}) catch {
                return error.InsertFailed;
            };
        }

        return inode_id;
    }

    /// Create a symlink by path.
    pub fn createSymlinkByPath(self: *Database, path: []const u8, target: []const u8) Error!u64 {
        const now_ts = Timespec.now();
        const name = std.fs.path.basename(path);

        self.db.exec(
            \\INSERT INTO inodes (parent_id, name, path, type, mode, symlink_target,
            \\    atime_sec, atime_nsec, mtime_sec, mtime_nsec, ctime_sec, ctime_nsec, btime_sec, btime_nsec)
            \\VALUES (0, ?, ?, 2, 511, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ,
            .{},
            .{
                name,       path,        target,     now_ts.sec,  now_ts.nsec, now_ts.sec, now_ts.nsec,
                now_ts.sec, now_ts.nsec, now_ts.sec, now_ts.nsec,
            },
        ) catch return error.InsertFailed;

        return @intCast(self.db.getLastInsertRowID());
    }

    /// Remove by path.
    pub fn removeByPath(self: *Database, path: []const u8) Error!void {
        self.db.exec("DELETE FROM inodes WHERE path = ?", .{}, .{path}) catch {
            return error.DeleteFailed;
        };
    }

    /// Read file by path.
    pub fn readByPath(self: *Database, path: []const u8, offset: i64, buf: []u8) Error!usize {
        const query =
            \\SELECT fd.data FROM file_data fd
            \\JOIN inodes i ON i.id = fd.inode_id
            \\WHERE i.path = ?
        ;
        const row = self.db.oneAlloc(struct { data: []const u8 }, self.allocator, query, .{}, .{path}) catch {
            return error.QueryFailed;
        } orelse return error.NotFound;
        defer self.allocator.free(row.data);

        if (offset < 0 or @as(u64, @intCast(offset)) >= row.data.len) {
            return 0;
        }

        const start: usize = @intCast(offset);
        const available = row.data.len - start;
        const to_copy = @min(available, buf.len);
        @memcpy(buf[0..to_copy], row.data[start..][0..to_copy]);
        return to_copy;
    }

    /// Write file by path.
    pub fn writeByPath(self: *Database, path: []const u8, offset: i64, data: []const u8) Error!usize {
        // Get inode_id from path
        const id_row = self.db.one(struct { id: u64 }, "SELECT id FROM inodes WHERE path = ?", .{}, .{path}) catch {
            return error.QueryFailed;
        } orelse return error.NotFound;

        return self.write(id_row.id, offset, data);
    }

    /// Read symlink by path.
    pub fn readSymlinkByPath(self: *Database, path: []const u8, buf: []u8) Error!usize {
        const row = (self.db.oneAlloc(
            struct { symlink_target: ?[]const u8 },
            self.allocator,
            "SELECT symlink_target FROM inodes WHERE path = ?",
            .{},
            .{path},
        ) catch return error.QueryFailed) orelse return error.NotFound;
        defer if (row.symlink_target) |t| self.allocator.free(t);

        const target = row.symlink_target orelse return error.NotASymlink;
        const to_copy = @min(target.len, buf.len);
        @memcpy(buf[0..to_copy], target[0..to_copy]);
        return to_copy;
    }

    /// Read directory by path (parent_id=0 for path-indexed entries).
    pub fn readDirByPath(self: *Database, parent_path: []const u8, allocator: std.mem.Allocator) Error![]DirEntry {
        // For path-based entries, we need to find all entries whose path starts with parent_path/
        const prefix = if (parent_path.len == 0 or std.mem.eql(u8, parent_path, "/"))
            ""
        else
            parent_path;

        var stmt = self.db.prepare("SELECT id, name, path, type FROM inodes WHERE path LIKE ? || '/%' AND path NOT LIKE ? || '/%/%'") catch return error.QueryFailed;
        defer stmt.deinit();

        var entries: std.ArrayList(DirEntry) = .{};
        errdefer {
            for (entries.items) |entry| allocator.free(entry.name);
            entries.deinit(allocator);
        }

        var iter = stmt.iterator(struct { id: u64, name: []const u8, path: []const u8, type: u8 }, .{ prefix, prefix }) catch return error.QueryFailed;

        while (iter.nextAlloc(allocator, .{}) catch return error.QueryFailed) |row| {
            defer allocator.free(row.path);
            entries.append(allocator, .{
                .inode_id = row.id,
                .name = row.name,
                .item_type = @enumFromInt(row.type),
            }) catch {
                allocator.free(row.name);
                return error.OutOfMemory;
            };
        }

        return entries.toOwnedSlice(allocator) catch return error.OutOfMemory;
    }

    // =========================================================================
    // Whiteout operations
    // =========================================================================

    /// Add a whiteout (mark path as deleted).
    pub fn addWhiteout(self: *Database, path: []const u8) Error!void {
        self.db.exec("INSERT OR IGNORE INTO whiteouts (path) VALUES (?)", .{}, .{path}) catch {
            return error.InsertFailed;
        };
    }

    /// Remove a whiteout.
    pub fn removeWhiteout(self: *Database, path: []const u8) Error!void {
        self.db.exec("DELETE FROM whiteouts WHERE path = ?", .{}, .{path}) catch {
            return error.DeleteFailed;
        };
    }

    /// Check if path is whited out.
    pub fn isWhiteout(self: *Database, path: []const u8) Error!bool {
        const row = self.db.one(struct { exists: u32 }, "SELECT 1 FROM whiteouts WHERE path = ?", .{}, .{path}) catch {
            return error.QueryFailed;
        };
        return row != null;
    }

    /// Get all whiteouts.
    pub fn getAllWhiteouts(self: *Database, allocator: std.mem.Allocator) Error![][]const u8 {
        var stmt = self.db.prepare("SELECT path FROM whiteouts") catch return error.QueryFailed;
        defer stmt.deinit();

        var paths: std.ArrayList([]const u8) = .{};
        errdefer {
            for (paths.items) |p| allocator.free(p);
            paths.deinit(allocator);
        }

        var iter = stmt.iterator(struct { path: []const u8 }, .{}) catch return error.QueryFailed;

        while (iter.nextAlloc(allocator, .{}) catch return error.QueryFailed) |row| {
            paths.append(allocator, row.path) catch {
                allocator.free(row.path);
                return error.OutOfMemory;
            };
        }

        return paths.toOwnedSlice(allocator) catch return error.OutOfMemory;
    }

    /// Get all paths in the overlay.
    pub fn getAllPaths(self: *Database, allocator: std.mem.Allocator) Error![][]const u8 {
        var stmt = self.db.prepare("SELECT path FROM inodes WHERE path IS NOT NULL") catch return error.QueryFailed;
        defer stmt.deinit();

        var paths: std.ArrayList([]const u8) = .{};
        errdefer {
            for (paths.items) |p| allocator.free(p);
            paths.deinit(allocator);
        }

        var iter = stmt.iterator(struct { path: []const u8 }, .{}) catch return error.QueryFailed;

        while (iter.nextAlloc(allocator, .{}) catch return error.QueryFailed) |row| {
            paths.append(allocator, row.path) catch {
                allocator.free(row.path);
                return error.OutOfMemory;
            };
        }

        return paths.toOwnedSlice(allocator) catch return error.OutOfMemory;
    }

    /// Clear all overlay data (for reject operation).
    pub fn clearOverlay(self: *Database) Error!void {
        self.db.exec("DELETE FROM inodes WHERE path IS NOT NULL", .{}, .{}) catch return error.DeleteFailed;
        self.db.exec("DELETE FROM whiteouts", .{}, .{}) catch return error.DeleteFailed;
    }
};
