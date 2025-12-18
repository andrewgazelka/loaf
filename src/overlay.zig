const std = @import("std");
const db = @import("db.zig");
const err = @import("error.zig");

pub const Database = db.Database;
pub const Attrs = db.Attrs;
pub const ItemType = db.ItemType;
pub const DirEntry = db.DirEntry;
pub const Error = db.Error;

/// Overlay filesystem - SQLite upper layer over real filesystem.
/// Writes go to SQLite, reads check SQLite first then fall through to real fs.
pub const Overlay = struct {
    database: Database,
    allocator: std.mem.Allocator,
    base_path: []const u8,

    pub fn init(allocator: std.mem.Allocator, db_path: [:0]const u8, base_path: []const u8) Error!*Overlay {
        const overlay = allocator.create(Overlay) catch return error.OutOfMemory;
        errdefer allocator.destroy(overlay);

        const base_path_copy = allocator.dupe(u8, base_path) catch return error.OutOfMemory;
        errdefer allocator.free(base_path_copy);

        overlay.* = .{
            .database = try Database.init(.{
                .path = db_path,
                .allocator = allocator,
            }),
            .allocator = allocator,
            .base_path = base_path_copy,
        };

        // Store base path in database for persistence (only if non-empty)
        if (base_path.len > 0) {
            overlay.database.setBasePath(base_path) catch |e| {
                overlay.database.deinit();
                allocator.free(base_path_copy);
                return e;
            };
        }

        return overlay;
    }

    pub fn deinit(self: *Overlay) void {
        self.database.deinit();
        self.allocator.free(self.base_path);
        self.allocator.destroy(self);
    }

    /// Resolve a path relative to base_path on the real filesystem.
    fn realPath(self: *Overlay, rel_path: []const u8) ![]const u8 {
        if (rel_path.len == 0 or std.mem.eql(u8, rel_path, "/")) {
            return self.allocator.dupe(u8, self.base_path);
        }
        const clean_path = if (rel_path[0] == '/') rel_path[1..] else rel_path;
        return std.fmt.allocPrint(self.allocator, "{s}/{s}", .{ self.base_path, clean_path }) catch return error.OutOfMemory;
    }

    /// Check if a path exists in the real filesystem.
    fn existsInReal(self: *Overlay, rel_path: []const u8) bool {
        const full_path = self.realPath(rel_path) catch return false;
        defer self.allocator.free(full_path);
        std.fs.cwd().access(full_path, .{}) catch return false;
        return true;
    }

    /// Check if path is whited out (deleted in overlay).
    fn isWhitedOut(self: *Overlay, rel_path: []const u8) bool {
        return self.database.isWhiteout(rel_path) catch false;
    }

    /// Get attributes - check overlay first, then real fs.
    pub fn getAttrs(self: *Overlay, rel_path: []const u8) Error!Attrs {
        // Check for whiteout first
        if (self.isWhitedOut(rel_path)) {
            return error.NotFound;
        }

        // Check overlay (SQLite)
        if (self.database.getAttrsByPath(rel_path)) |attrs| {
            return attrs;
        } else |e| {
            if (e != error.NotFound) return e;
        }

        // Fall through to real filesystem
        const full_path = self.realPath(rel_path) catch return error.OutOfMemory;
        defer self.allocator.free(full_path);

        const stat = std.fs.cwd().statFile(full_path) catch return error.NotFound;
        return statToAttrs(stat, rel_path);
    }

    /// Lookup child in directory - merge overlay + real fs - whiteouts.
    pub fn lookup(self: *Overlay, parent_path: []const u8, name: []const u8) Error![]const u8 {
        const child_path = if (parent_path.len == 0 or std.mem.eql(u8, parent_path, "/"))
            self.allocator.dupe(u8, name) catch return error.OutOfMemory
        else
            std.fmt.allocPrint(self.allocator, "{s}/{s}", .{ parent_path, name }) catch return error.OutOfMemory;

        // Check for whiteout
        if (self.isWhitedOut(child_path)) {
            self.allocator.free(child_path);
            return error.NotFound;
        }

        // Check overlay
        if (self.database.existsByPath(child_path)) {
            return child_path;
        }

        // Check real filesystem
        const full_path = self.realPath(child_path) catch {
            self.allocator.free(child_path);
            return error.OutOfMemory;
        };
        defer self.allocator.free(full_path);

        std.fs.cwd().access(full_path, .{}) catch {
            self.allocator.free(child_path);
            return error.NotFound;
        };

        return child_path;
    }

    /// Create file/directory - always in overlay.
    pub fn create(self: *Overlay, parent_path: []const u8, name: []const u8, item_type: ItemType, mode: u32) Error![]const u8 {
        const child_path = if (parent_path.len == 0 or std.mem.eql(u8, parent_path, "/"))
            self.allocator.dupe(u8, name) catch return error.OutOfMemory
        else
            std.fmt.allocPrint(self.allocator, "{s}/{s}", .{ parent_path, name }) catch return error.OutOfMemory;
        errdefer self.allocator.free(child_path);

        // Remove whiteout if exists
        self.database.removeWhiteout(child_path) catch {};

        // Check if already exists
        if (self.database.existsByPath(child_path)) {
            return error.AlreadyExists;
        }

        // Check real fs (and not whited out)
        if (!self.isWhitedOut(child_path) and self.existsInReal(child_path)) {
            return error.AlreadyExists;
        }

        // Create in overlay
        _ = try self.database.createByPath(child_path, item_type, mode);

        return child_path;
    }

    /// Create symlink - always in overlay.
    pub fn createSymlink(self: *Overlay, parent_path: []const u8, name: []const u8, target: []const u8) Error![]const u8 {
        const child_path = if (parent_path.len == 0 or std.mem.eql(u8, parent_path, "/"))
            self.allocator.dupe(u8, name) catch return error.OutOfMemory
        else
            std.fmt.allocPrint(self.allocator, "{s}/{s}", .{ parent_path, name }) catch return error.OutOfMemory;
        errdefer self.allocator.free(child_path);

        // Remove whiteout if exists
        self.database.removeWhiteout(child_path) catch {};

        // Check if already exists
        if (self.database.existsByPath(child_path)) {
            return error.AlreadyExists;
        }

        if (!self.isWhitedOut(child_path) and self.existsInReal(child_path)) {
            return error.AlreadyExists;
        }

        // Create in overlay
        _ = try self.database.createSymlinkByPath(child_path, target);

        return child_path;
    }

    /// Remove - create whiteout if real, delete if overlay-only.
    pub fn remove(self: *Overlay, rel_path: []const u8) Error!void {
        const in_overlay = self.database.existsByPath(rel_path);
        const in_real = !self.isWhitedOut(rel_path) and self.existsInReal(rel_path);

        if (!in_overlay and !in_real) {
            return error.NotFound;
        }

        // If in overlay, remove from overlay
        if (in_overlay) {
            try self.database.removeByPath(rel_path);
        }

        // If in real fs, create whiteout
        if (in_real) {
            try self.database.addWhiteout(rel_path);
        }
    }

    /// Read file - check overlay first, then real fs.
    pub fn read(self: *Overlay, rel_path: []const u8, offset: i64, buf: []u8) Error!usize {
        if (self.isWhitedOut(rel_path)) {
            return error.NotFound;
        }

        // Check overlay first
        if (self.database.readByPath(rel_path, offset, buf)) |bytes| {
            return bytes;
        } else |e| {
            if (e != error.NotFound) return e;
        }

        // Fall through to real filesystem
        const full_path = self.realPath(rel_path) catch return error.OutOfMemory;
        defer self.allocator.free(full_path);

        const file = std.fs.cwd().openFile(full_path, .{}) catch return error.NotFound;
        defer file.close();

        if (offset > 0) {
            file.seekTo(@intCast(offset)) catch return error.IoError;
        }

        return file.read(buf) catch return error.IoError;
    }

    /// Write file - copy-up to overlay if needed, then write.
    pub fn write(self: *Overlay, rel_path: []const u8, offset: i64, data: []const u8) Error!usize {
        if (self.isWhitedOut(rel_path)) {
            return error.NotFound;
        }

        // If not in overlay, do copy-up from real fs
        if (!self.database.existsByPath(rel_path)) {
            try self.copyUp(rel_path);
        }

        // Write to overlay
        return self.database.writeByPath(rel_path, offset, data);
    }

    /// Copy file from real fs to overlay (copy-up operation).
    fn copyUp(self: *Overlay, rel_path: []const u8) Error!void {
        const full_path = self.realPath(rel_path) catch return error.OutOfMemory;
        defer self.allocator.free(full_path);

        const stat = std.fs.cwd().statFile(full_path) catch return error.NotFound;

        // Create entry in overlay
        const inode_id = try self.database.createByPath(rel_path, .file, @intCast(stat.mode & 0o777));

        // Copy content
        const file = std.fs.cwd().openFile(full_path, .{}) catch return error.NotFound;
        defer file.close();

        var buf: [8192]u8 = undefined;
        var total_written: usize = 0;

        while (true) {
            const bytes_read = file.read(&buf) catch return error.IoError;
            if (bytes_read == 0) break;

            _ = self.database.write(inode_id, @intCast(total_written), buf[0..bytes_read]) catch return error.IoError;
            total_written += bytes_read;
        }
    }

    /// Read symlink - check overlay first, then real fs.
    pub fn readSymlink(self: *Overlay, rel_path: []const u8, buf: []u8) Error!usize {
        if (self.isWhitedOut(rel_path)) {
            return error.NotFound;
        }

        // Check overlay first
        if (self.database.readSymlinkByPath(rel_path, buf)) |len| {
            return len;
        } else |e| {
            if (e != error.NotFound) return e;
        }

        // Fall through to real filesystem
        const full_path = self.realPath(rel_path) catch return error.OutOfMemory;
        defer self.allocator.free(full_path);

        const link = std.fs.cwd().readLink(full_path, buf) catch return error.NotFound;
        return link.len;
    }

    /// List directory - merge overlay + real fs - whiteouts.
    pub fn readDir(self: *Overlay, rel_path: []const u8) Error![]DirEntry {
        var entries = std.StringHashMap(DirEntry).init(self.allocator);
        defer entries.deinit();

        // Get entries from overlay
        const overlay_entries = self.database.readDirByPath(rel_path, self.allocator) catch |e| {
            if (e != error.NotFound) return e;
            @as([]DirEntry, &.{});
        };
        defer {
            for (overlay_entries) |entry| {
                self.allocator.free(entry.name);
            }
            self.allocator.free(overlay_entries);
        }

        for (overlay_entries) |entry| {
            const name_copy = self.allocator.dupe(u8, entry.name) catch return error.OutOfMemory;
            entries.put(name_copy, .{
                .inode_id = entry.inode_id,
                .name = name_copy,
                .item_type = entry.item_type,
            }) catch return error.OutOfMemory;
        }

        // Get entries from real fs (if not whited out)
        if (!self.isWhitedOut(rel_path)) {
            const full_path = self.realPath(rel_path) catch return error.OutOfMemory;
            defer self.allocator.free(full_path);

            if (std.fs.cwd().openDir(full_path, .{ .iterate = true })) |*dir| {
                defer dir.close();
                var iter = dir.iterate();
                while (iter.next() catch null) |entry| {
                    // Skip if already in overlay or whited out
                    if (entries.contains(entry.name)) continue;

                    const child_path = std.fmt.allocPrint(self.allocator, "{s}/{s}", .{ rel_path, entry.name }) catch continue;
                    defer self.allocator.free(child_path);

                    if (self.isWhitedOut(child_path)) continue;

                    const name_copy = self.allocator.dupe(u8, entry.name) catch continue;
                    entries.put(name_copy, .{
                        .inode_id = 0, // Real fs entries don't have inode IDs
                        .name = name_copy,
                        .item_type = kindToItemType(entry.kind),
                    }) catch {
                        self.allocator.free(name_copy);
                        continue;
                    };
                }
            } else |_| {}
        }

        // Convert to slice
        var result = self.allocator.alloc(DirEntry, entries.count()) catch return error.OutOfMemory;
        var i: usize = 0;
        var iter = entries.iterator();
        while (iter.next()) |kv| {
            result[i] = kv.value_ptr.*;
            i += 1;
        }

        return result;
    }

    pub fn freeDirEntries(self: *Overlay, entries: []DirEntry) void {
        for (entries) |entry| {
            self.allocator.free(entry.name);
        }
        self.allocator.free(entries);
    }

    pub fn sync(self: *Overlay) Error!void {
        return self.database.sync();
    }

    // =========================================================================
    // Diff / Accept / Reject operations
    // =========================================================================

    pub const Change = struct {
        path: []const u8,
        change_type: ChangeType,
    };

    pub const ChangeType = enum {
        added,
        modified,
        deleted,
    };

    /// Get list of all changes in the overlay.
    pub fn getChanges(self: *Overlay) Error![]Change {
        var changes: std.ArrayList(Change) = .empty;
        errdefer {
            for (changes.items) |c| self.allocator.free(c.path);
            changes.deinit(self.allocator);
        }

        // Get all modified/created files from overlay
        const modified = try self.database.getAllPaths(self.allocator);
        defer {
            for (modified) |p| self.allocator.free(p);
            self.allocator.free(modified);
        }

        for (modified) |path| {
            const path_copy = self.allocator.dupe(u8, path) catch return error.OutOfMemory;
            const change_type: ChangeType = if (self.existsInReal(path)) .modified else .added;
            changes.append(self.allocator, .{ .path = path_copy, .change_type = change_type }) catch return error.OutOfMemory;
        }

        // Get all whiteouts (deleted files)
        const whiteouts = try self.database.getAllWhiteouts(self.allocator);
        defer {
            for (whiteouts) |p| self.allocator.free(p);
            self.allocator.free(whiteouts);
        }

        for (whiteouts) |path| {
            const path_copy = self.allocator.dupe(u8, path) catch return error.OutOfMemory;
            changes.append(self.allocator, .{ .path = path_copy, .change_type = .deleted }) catch return error.OutOfMemory;
        }

        return changes.toOwnedSlice(self.allocator) catch return error.OutOfMemory;
    }

    pub fn freeChanges(self: *Overlay, changes: []Change) void {
        for (changes) |c| self.allocator.free(c.path);
        self.allocator.free(changes);
    }

    /// Apply all changes to real filesystem.
    pub fn accept(self: *Overlay) Error!void {
        const changes = try self.getChanges();
        defer self.freeChanges(changes);

        for (changes) |change| {
            switch (change.change_type) {
                .added, .modified => {
                    // Copy from overlay to real fs
                    try self.applyFile(change.path);
                },
                .deleted => {
                    // Delete from real fs
                    const full_path = try self.realPath(change.path);
                    defer self.allocator.free(full_path);
                    std.fs.cwd().deleteFile(full_path) catch |e| {
                        if (e != error.FileNotFound) {
                            std.fs.cwd().deleteTree(full_path) catch {};
                        }
                    };
                },
            }
        }

        // Clear overlay
        try self.database.clearOverlay();
    }

    fn applyFile(self: *Overlay, rel_path: []const u8) Error!void {
        const full_path = try self.realPath(rel_path);
        defer self.allocator.free(full_path);

        const attrs = self.database.getAttrsByPath(rel_path) catch return;

        if (attrs.item_type == @intFromEnum(ItemType.directory)) {
            std.fs.cwd().makePath(full_path) catch return error.IoError;
        } else if (attrs.item_type == @intFromEnum(ItemType.symlink)) {
            var buf: [4096]u8 = undefined;
            const len = try self.database.readSymlinkByPath(rel_path, &buf);
            const target = buf[0..len];

            std.fs.cwd().deleteFile(full_path) catch {};
            std.fs.cwd().symLink(target, full_path, .{}) catch return error.IoError;
        } else {
            // Regular file
            const parent_path = std.fs.path.dirname(full_path) orelse ".";
            std.fs.cwd().makePath(parent_path) catch {};

            // Use createFileAbsolute for absolute paths
            const file = if (full_path.len > 0 and full_path[0] == '/')
                std.fs.createFileAbsolute(full_path, .{}) catch return error.IoError
            else
                std.fs.cwd().createFile(full_path, .{}) catch return error.IoError;
            defer file.close();

            var offset: usize = 0;
            var buf: [8192]u8 = undefined;
            while (true) {
                const bytes = self.database.readByPath(rel_path, @intCast(offset), &buf) catch break;
                if (bytes == 0) break;
                file.writeAll(buf[0..bytes]) catch return error.IoError;
                offset += bytes;
            }
        }
    }

    /// Discard all changes.
    pub fn reject(self: *Overlay) Error!void {
        try self.database.clearOverlay();
    }
};

// Helper functions

fn statToAttrs(stat: std.fs.File.Stat, _: []const u8) Attrs {
    const now = db.Timespec.now();
    return Attrs{
        .file_id = 0,
        .parent_id = 0,
        .item_type = switch (stat.kind) {
            .directory => @intFromEnum(ItemType.directory),
            .sym_link => @intFromEnum(ItemType.symlink),
            else => @intFromEnum(ItemType.file),
        },
        .mode = @intCast(stat.mode & 0o777),
        .uid = 0,
        .gid = 0,
        .size = @intCast(stat.size),
        .alloc_size = @intCast(stat.size),
        .link_count = 1,
        .flags = 0,
        .atime_sec = now.sec,
        .atime_nsec = now.nsec,
        .mtime_sec = @intCast(@divFloor(stat.mtime, std.time.ns_per_s)),
        .mtime_nsec = @intCast(@mod(stat.mtime, std.time.ns_per_s)),
        .ctime_sec = @intCast(@divFloor(stat.ctime, std.time.ns_per_s)),
        .ctime_nsec = @intCast(@mod(stat.ctime, std.time.ns_per_s)),
        .btime_sec = now.sec,
        .btime_nsec = now.nsec,
    };
}

fn kindToItemType(kind: std.fs.Dir.Entry.Kind) ItemType {
    return switch (kind) {
        .directory => .directory,
        .sym_link => .symlink,
        else => .file,
    };
}
