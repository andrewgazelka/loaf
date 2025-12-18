const std = @import("std");
const db_mod = @import("db.zig");
const fs_mod = @import("fs.zig");

const Database = db_mod.Database;
const ItemType = db_mod.ItemType;
const Attrs = fs_mod.Attrs;
const DirEntry = fs_mod.DirEntry;

/// OverlayFs provides inode-based operations over an overlay database + real filesystem.
/// Reads check SQLite first, fall through to real filesystem.
/// Writes go to SQLite only. Deletes create whiteouts.
pub const OverlayFs = struct {
    allocator: std.mem.Allocator,
    database: *Database,
    base_path: []const u8,
    /// Maps inode IDs to paths. Inode 1 = "/" (root)
    inode_to_path: std.AutoHashMap(u64, []const u8),
    /// Maps paths to inode IDs
    path_to_inode: std.StringHashMap(u64),
    /// Next synthetic inode ID for real filesystem entries
    next_inode: u64,

    pub const Error = error{
        NotFound,
        NotADirectory,
        NotAFile,
        NotASymlink,
        AlreadyExists,
        NotEmpty,
        IoError,
        OutOfMemory,
        InvalidPath,
        QueryFailed,
        InsertFailed,
    };

    pub fn init(allocator: std.mem.Allocator, db_path: [:0]const u8, base_path: []const u8) Error!*OverlayFs {
        const self = allocator.create(OverlayFs) catch return error.OutOfMemory;
        errdefer allocator.destroy(self);

        const db_ptr = allocator.create(Database) catch return error.OutOfMemory;
        errdefer allocator.destroy(db_ptr);

        db_ptr.* = Database.init(.{ .path = db_path, .allocator = allocator }) catch return error.IoError;
        errdefer db_ptr.deinit();

        // Store base path in overlay config
        db_ptr.setBasePath(base_path) catch {};

        const base_path_copy = allocator.dupe(u8, base_path) catch return error.OutOfMemory;

        self.* = .{
            .allocator = allocator,
            .database = db_ptr,
            .base_path = base_path_copy,
            .inode_to_path = std.AutoHashMap(u64, []const u8).init(allocator),
            .path_to_inode = std.StringHashMap(u64).init(allocator),
            .next_inode = 1000000, // Start high to avoid conflicts with DB inodes
        };

        // Register root
        self.registerPath(1, "/") catch return error.OutOfMemory;

        return self;
    }

    pub fn deinit(self: *OverlayFs) void {
        // Free all stored paths
        var it = self.inode_to_path.valueIterator();
        while (it.next()) |path| {
            self.allocator.free(path.*);
        }
        self.inode_to_path.deinit();
        self.path_to_inode.deinit();
        self.allocator.free(self.base_path);
        self.database.deinit();
        self.allocator.destroy(self);
    }

    fn registerPath(self: *OverlayFs, inode: u64, path: []const u8) Error!void {
        const path_copy = self.allocator.dupe(u8, path) catch return error.OutOfMemory;
        self.inode_to_path.put(inode, path_copy) catch {
            self.allocator.free(path_copy);
            return error.OutOfMemory;
        };
        self.path_to_inode.put(path_copy, inode) catch return error.OutOfMemory;
    }

    fn getPath(self: *OverlayFs, inode: u64) ?[]const u8 {
        return self.inode_to_path.get(inode);
    }

    fn getOrCreateInode(self: *OverlayFs, path: []const u8) Error!u64 {
        if (self.path_to_inode.get(path)) |inode| {
            return inode;
        }
        const inode = self.next_inode;
        self.next_inode += 1;
        try self.registerPath(inode, path);
        return inode;
    }

    fn joinPath(self: *OverlayFs, parent_path: []const u8, name: []const u8) Error![]u8 {
        if (std.mem.eql(u8, parent_path, "/")) {
            return std.fmt.allocPrint(self.allocator, "/{s}", .{name}) catch return error.OutOfMemory;
        }
        return std.fmt.allocPrint(self.allocator, "{s}/{s}", .{ parent_path, name }) catch return error.OutOfMemory;
    }

    fn realPath(self: *OverlayFs, rel_path: []const u8) Error![]u8 {
        if (std.mem.eql(u8, rel_path, "/")) {
            return self.allocator.dupe(u8, self.base_path) catch return error.OutOfMemory;
        }
        // rel_path starts with /, so skip it
        const trimmed = if (rel_path.len > 0 and rel_path[0] == '/') rel_path[1..] else rel_path;
        return std.fmt.allocPrint(self.allocator, "{s}/{s}", .{ self.base_path, trimmed }) catch return error.OutOfMemory;
    }

    pub fn getRootId(self: *OverlayFs) u64 {
        _ = self;
        return 1;
    }

    pub fn getAttrs(self: *OverlayFs, inode_id: u64) Error!Attrs {
        const path = self.getPath(inode_id) orelse return error.NotFound;

        // Check whiteout first
        if ((self.database.isWhiteout(path) catch false)) {
            return error.NotFound;
        }

        // Check overlay database
        if (self.database.getAttrsByPath(path)) |db_attrs| {
            return Attrs{
                .file_id = inode_id,
                .parent_id = 0, // TODO
                .item_type = db_attrs.item_type,
                .mode = db_attrs.mode,
                .uid = db_attrs.uid,
                .gid = db_attrs.gid,
                .size = db_attrs.size,
                .alloc_size = db_attrs.size,
                .link_count = 1,
                .flags = 0,
                .atime_sec = db_attrs.atime_sec,
                .atime_nsec = db_attrs.atime_nsec,
                .mtime_sec = db_attrs.mtime_sec,
                .mtime_nsec = db_attrs.mtime_nsec,
                .ctime_sec = db_attrs.ctime_sec,
                .ctime_nsec = db_attrs.ctime_nsec,
                .btime_sec = db_attrs.ctime_sec,
                .btime_nsec = db_attrs.ctime_nsec,
            };
        } else |_| {}

        // Fall through to real filesystem
        const real = try self.realPath(path);
        defer self.allocator.free(real);

        const stat = std.fs.cwd().statFile(real) catch return error.NotFound;

        const item_type: u8 = switch (stat.kind) {
            .directory => @intFromEnum(ItemType.directory),
            .sym_link => @intFromEnum(ItemType.symlink),
            else => @intFromEnum(ItemType.file),
        };

        const atime_sec: i64 = @intCast(@divFloor(stat.atime, std.time.ns_per_s));
        const mtime_sec: i64 = @intCast(@divFloor(stat.mtime, std.time.ns_per_s));
        const ctime_sec: i64 = @intCast(@divFloor(stat.ctime, std.time.ns_per_s));

        return Attrs{
            .file_id = inode_id,
            .parent_id = 0,
            .item_type = item_type,
            .mode = @intCast(stat.mode),
            .uid = 0,
            .gid = 0,
            .size = stat.size,
            .alloc_size = stat.size,
            .link_count = 1,
            .flags = 0,
            .atime_sec = atime_sec,
            .atime_nsec = 0,
            .mtime_sec = mtime_sec,
            .mtime_nsec = 0,
            .ctime_sec = ctime_sec,
            .ctime_nsec = 0,
            .btime_sec = ctime_sec,
            .btime_nsec = 0,
        };
    }

    pub fn lookup(self: *OverlayFs, parent_id: u64, name: []const u8) Error!u64 {
        const parent_path = self.getPath(parent_id) orelse return error.NotFound;
        const child_path = try self.joinPath(parent_path, name);
        defer self.allocator.free(child_path);

        // Check whiteout
        if ((self.database.isWhiteout(child_path) catch false)) {
            return error.NotFound;
        }

        // Check overlay
        if (self.database.existsByPath(child_path)) {
            return try self.getOrCreateInode(child_path);
        }

        // Check real filesystem
        const real = try self.realPath(child_path);
        defer self.allocator.free(real);

        _ = std.fs.cwd().statFile(real) catch return error.NotFound;

        return try self.getOrCreateInode(child_path);
    }

    pub fn create(self: *OverlayFs, parent_id: u64, name: []const u8, item_type: ItemType, mode: u32) Error!u64 {
        const parent_path = self.getPath(parent_id) orelse return error.NotFound;
        const child_path = try self.joinPath(parent_path, name);
        defer self.allocator.free(child_path);

        // Remove any whiteout
        self.database.removeWhiteout(child_path) catch {};

        // Create in overlay
        _ = self.database.createByPath(child_path, item_type, mode) catch return error.InsertFailed;

        return try self.getOrCreateInode(child_path);
    }

    pub fn createSymlink(self: *OverlayFs, parent_id: u64, name: []const u8, target: []const u8) Error!u64 {
        const parent_path = self.getPath(parent_id) orelse return error.NotFound;
        const child_path = try self.joinPath(parent_path, name);
        defer self.allocator.free(child_path);

        // Remove any whiteout
        self.database.removeWhiteout(child_path) catch {};

        // Create symlink in overlay
        _ = self.database.createSymlinkByPath(child_path, target) catch return error.InsertFailed;

        return try self.getOrCreateInode(child_path);
    }

    pub fn remove(self: *OverlayFs, parent_id: u64, inode_id: u64) Error!void {
        _ = parent_id;
        const path = self.getPath(inode_id) orelse return error.NotFound;

        // If it's in overlay, delete from overlay
        if (self.database.existsByPath(path)) {
            self.database.removeByPath(path) catch {};
        }

        // Check if it exists in real filesystem
        const real = try self.realPath(path);
        defer self.allocator.free(real);

        if (std.fs.cwd().statFile(real)) |_| {
            // Create whiteout to hide real file
            self.database.addWhiteout(path) catch return error.InsertFailed;
        } else |_| {}
    }

    pub fn read(self: *OverlayFs, inode_id: u64, offset: i64, buf: []u8) Error!usize {
        const path = self.getPath(inode_id) orelse return error.NotFound;

        // Check whiteout
        if ((self.database.isWhiteout(path) catch false)) {
            return error.NotFound;
        }

        // Check overlay first
        if (self.database.readByPath(path, @intCast(offset), buf)) |bytes| {
            return bytes;
        } else |_| {}

        // Fall through to real filesystem
        const real = try self.realPath(path);
        defer self.allocator.free(real);

        const file = std.fs.cwd().openFile(real, .{}) catch return error.NotFound;
        defer file.close();

        if (offset > 0) {
            file.seekTo(@intCast(offset)) catch return error.IoError;
        }

        return file.read(buf) catch return error.IoError;
    }

    pub fn write(self: *OverlayFs, inode_id: u64, offset: i64, data: []const u8) Error!usize {
        const path = self.getPath(inode_id) orelse return error.NotFound;

        // If not in overlay yet, copy-up from real filesystem first
        if (!self.database.existsByPath(path)) {
            // Create empty file in overlay
            _ = self.database.createByPath(path, .file, 0o644) catch return error.InsertFailed;

            // Copy existing content from real filesystem
            const real = try self.realPath(path);
            defer self.allocator.free(real);

            if (std.fs.cwd().openFile(real, .{})) |file| {
                defer file.close();
                var copy_buf: [8192]u8 = undefined;
                var copy_offset: usize = 0;
                while (true) {
                    const bytes = file.read(&copy_buf) catch break;
                    if (bytes == 0) break;
                    _ = self.database.writeByPath(path, @intCast(copy_offset), copy_buf[0..bytes]) catch break;
                    copy_offset += bytes;
                }
            } else |_| {}
        }

        // Write to overlay
        return self.database.writeByPath(path, @intCast(offset), data) catch return error.IoError;
    }

    pub fn readSymlink(self: *OverlayFs, inode_id: u64, buf: []u8) Error!usize {
        const path = self.getPath(inode_id) orelse return error.NotFound;

        // Check overlay first
        if (self.database.readSymlinkByPath(path, buf)) |len| {
            return len;
        } else |_| {}

        // Fall through to real filesystem
        const real = try self.realPath(path);
        defer self.allocator.free(real);

        const target = std.fs.cwd().readLink(real, buf) catch return error.NotFound;
        return target.len;
    }

    pub fn readDir(self: *OverlayFs, dir_inode_id: u64) Error![]DirEntry {
        const dir_path = self.getPath(dir_inode_id) orelse return error.NotFound;

        var entries: std.ArrayList(DirEntry) = .empty;
        errdefer {
            for (entries.items) |e| self.allocator.free(e.name);
            entries.deinit(self.allocator);
        }

        var seen = std.StringHashMap(void).init(self.allocator);
        defer seen.deinit();

        // Get entries from overlay
        const overlay_entries = self.database.readDirByPath(dir_path, self.allocator) catch &[_]db_mod.DirEntry{};
        defer {
            for (overlay_entries) |e| self.allocator.free(e.name);
            self.allocator.free(overlay_entries);
        }

        for (overlay_entries) |e| {
            const child_path = self.joinPath(dir_path, e.name) catch continue;
            defer self.allocator.free(child_path);

            // Skip if whiteout
            if ((self.database.isWhiteout(child_path) catch false)) continue;

            const inode = self.getOrCreateInode(child_path) catch continue;
            const name_copy = self.allocator.dupe(u8, e.name) catch continue;

            entries.append(self.allocator, .{
                .inode_id = inode,
                .name = name_copy,
                .item_type = e.item_type,
            }) catch {
                self.allocator.free(name_copy);
                continue;
            };

            seen.put(e.name, {}) catch {};
        }

        // Get entries from real filesystem
        const real = self.realPath(dir_path) catch return entries.toOwnedSlice(self.allocator) catch return error.OutOfMemory;
        defer self.allocator.free(real);

        var dir = std.fs.cwd().openDir(real, .{ .iterate = true }) catch
            return entries.toOwnedSlice(self.allocator) catch return error.OutOfMemory;
        defer dir.close();

        var iter = dir.iterate();
        while (iter.next() catch null) |entry| {
            // Skip if already in overlay or whiteout
            if (seen.contains(entry.name)) continue;

            const child_path = self.joinPath(dir_path, entry.name) catch continue;
            defer self.allocator.free(child_path);

            if ((self.database.isWhiteout(child_path) catch false)) continue;

            const inode = self.getOrCreateInode(child_path) catch continue;
            const name_copy = self.allocator.dupe(u8, entry.name) catch continue;

            const item_type: ItemType = switch (entry.kind) {
                .directory => .directory,
                .sym_link => .symlink,
                else => .file,
            };

            entries.append(self.allocator, .{
                .inode_id = inode,
                .name = name_copy,
                .item_type = item_type,
            }) catch {
                self.allocator.free(name_copy);
                continue;
            };
        }

        return entries.toOwnedSlice(self.allocator) catch return error.OutOfMemory;
    }

    pub fn freeDirEntries(self: *OverlayFs, entries: []DirEntry) void {
        for (entries) |e| self.allocator.free(e.name);
        self.allocator.free(entries);
    }

    pub fn sync(self: *OverlayFs) Error!void {
        _ = self;
        // SQLite handles syncing automatically
    }

    pub fn rename(self: *OverlayFs, src_parent_id: u64, src_inode_id: u64, dst_parent_id: u64, dst_name: []const u8) Error!?u64 {
        const src_path = self.getPath(src_inode_id) orelse return error.NotFound;
        const dst_parent_path = self.getPath(dst_parent_id) orelse return error.NotFound;
        _ = src_parent_id;

        const dst_path = try self.joinPath(dst_parent_path, dst_name);
        defer self.allocator.free(dst_path);

        // Check if destination exists
        var replaced_id: ?u64 = null;
        if (self.path_to_inode.get(dst_path)) |existing| {
            replaced_id = existing;
        }

        // For overlay, we need to:
        // 1. Copy source to destination in overlay (if source is in overlay)
        // 2. Create whiteout for source (if source is in real FS)

        // If source is in overlay, move it
        if (self.database.existsByPath(src_path)) {
            // Read source content
            var buf: [65536]u8 = undefined;
            if (self.database.readByPath(src_path, 0, &buf)) |len| {
                // Create at destination
                _ = self.database.createByPath(dst_path, .file, 0o644) catch {};
                _ = self.database.writeByPath(dst_path, 0, buf[0..len]) catch {};
            } else |_| {
                // Maybe a directory
                _ = self.database.createByPath(dst_path, .directory, 0o755) catch {};
            }
            // Delete source from overlay
            self.database.removeByPath(src_path) catch {};
        } else {
            // Source is in real FS - copy to overlay, whiteout source
            const real = try self.realPath(src_path);
            defer self.allocator.free(real);

            const stat = std.fs.cwd().statFile(real) catch return error.NotFound;
            const item_type: ItemType = switch (stat.kind) {
                .directory => .directory,
                .sym_link => .symlink,
                else => .file,
            };

            _ = self.database.createByPath(dst_path, item_type, @intCast(stat.mode)) catch {};

            // Copy content for files
            if (item_type == .file) {
                if (std.fs.cwd().openFile(real, .{})) |file| {
                    defer file.close();
                    var buf: [8192]u8 = undefined;
                    var offset: usize = 0;
                    while (true) {
                        const bytes = file.read(&buf) catch break;
                        if (bytes == 0) break;
                        _ = self.database.writeByPath(dst_path, @intCast(offset), buf[0..bytes]) catch break;
                        offset += bytes;
                    }
                } else |_| {}
            }
        }

        // Create whiteout for source path if it exists in real FS
        const src_real = try self.realPath(src_path);
        defer self.allocator.free(src_real);
        if (std.fs.cwd().statFile(src_real)) |_| {
            self.database.addWhiteout(src_path) catch {};
        } else |_| {}

        // Update path mappings
        const dst_inode = try self.getOrCreateInode(dst_path);
        _ = dst_inode;

        return replaced_id;
    }
};
