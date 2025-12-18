const std = @import("std");
const db = @import("db.zig");
const err = @import("error.zig");

pub const Database = db.Database;
pub const Attrs = db.Attrs;
pub const ItemType = db.ItemType;
pub const DirEntry = db.DirEntry;
pub const Error = db.Error;

/// Opaque filesystem handle for FFI.
pub const Filesystem = struct {
    database: Database,
    allocator: std.mem.Allocator,

    pub fn init(allocator: std.mem.Allocator, path: [:0]const u8) Error!*Filesystem {
        const fs = allocator.create(Filesystem) catch return error.OutOfMemory;
        errdefer allocator.destroy(fs);

        fs.* = .{
            .database = try Database.init(.{
                .path = path,
                .allocator = allocator,
            }),
            .allocator = allocator,
        };

        return fs;
    }

    pub fn deinit(self: *Filesystem) void {
        self.database.deinit();
        self.allocator.destroy(self);
    }

    pub fn getRootId(self: *Filesystem) u64 {
        _ = self;
        return 1; // Root is always inode 1
    }

    pub fn getAttrs(self: *Filesystem, inode_id: u64) Error!Attrs {
        return self.database.getAttrs(inode_id);
    }

    pub fn lookup(self: *Filesystem, parent_id: u64, name: []const u8) Error!u64 {
        return self.database.lookup(parent_id, name);
    }

    pub fn create(self: *Filesystem, parent_id: u64, name: []const u8, item_type: ItemType, mode: u32) Error!u64 {
        // Check parent exists and is a directory
        const parent_attrs = try self.getAttrs(parent_id);
        if (parent_attrs.item_type != @intFromEnum(ItemType.directory)) {
            return error.NotDirectory;
        }

        // Check name doesn't already exist
        if (self.database.lookup(parent_id, name)) |_| {
            return error.AlreadyExists;
        } else |e| {
            if (e != error.NotFound) return e;
        }

        return self.database.create(parent_id, name, item_type, mode);
    }

    pub fn createSymlink(self: *Filesystem, parent_id: u64, name: []const u8, target: []const u8) Error!u64 {
        // Check parent exists and is a directory
        const parent_attrs = try self.getAttrs(parent_id);
        if (parent_attrs.item_type != @intFromEnum(ItemType.directory)) {
            return error.NotDirectory;
        }

        // Check name doesn't already exist
        if (self.database.lookup(parent_id, name)) |_| {
            return error.AlreadyExists;
        } else |e| {
            if (e != error.NotFound) return e;
        }

        return self.database.createSymlink(parent_id, name, target);
    }

    pub fn remove(self: *Filesystem, parent_id: u64, inode_id: u64) Error!void {
        _ = parent_id;

        // Check it exists
        const attrs = try self.getAttrs(inode_id);

        // If directory, check it's empty
        if (attrs.item_type == @intFromEnum(ItemType.directory)) {
            const entries = try self.database.readDir(inode_id, self.allocator);
            defer {
                for (entries) |entry| {
                    self.allocator.free(entry.name);
                }
                self.allocator.free(entries);
            }

            if (entries.len > 0) {
                return error.NotEmpty;
            }
        }

        return self.database.remove(inode_id);
    }

    pub fn rename(
        self: *Filesystem,
        src_parent_id: u64,
        src_inode_id: u64,
        dst_parent_id: u64,
        dst_name: []const u8,
    ) Error!?u64 {
        _ = src_parent_id;

        // Check destination parent is a directory
        const dst_parent_attrs = try self.getAttrs(dst_parent_id);
        if (dst_parent_attrs.item_type != @intFromEnum(ItemType.directory)) {
            return error.NotDirectory;
        }

        // Check if destination exists (for replacement)
        var replaced_id: ?u64 = null;
        if (self.database.lookup(dst_parent_id, dst_name)) |existing_id| {
            // Remove existing
            try self.remove(dst_parent_id, existing_id);
            replaced_id = existing_id;
        } else |e| {
            if (e != error.NotFound) return e;
        }

        // Do the rename
        try self.database.rename(src_inode_id, dst_parent_id, dst_name);

        return replaced_id;
    }

    pub fn read(self: *Filesystem, inode_id: u64, offset: i64, buf: []u8) Error!usize {
        // Check it's a file
        const attrs = try self.getAttrs(inode_id);
        if (attrs.item_type != @intFromEnum(ItemType.file)) {
            return error.IsDirectory;
        }

        return self.database.read(inode_id, offset, buf);
    }

    pub fn write(self: *Filesystem, inode_id: u64, offset: i64, data: []const u8) Error!usize {
        // Check it's a file
        const attrs = try self.getAttrs(inode_id);
        if (attrs.item_type != @intFromEnum(ItemType.file)) {
            return error.IsDirectory;
        }

        return self.database.write(inode_id, offset, data);
    }

    pub fn readSymlink(self: *Filesystem, inode_id: u64, buf: []u8) Error!usize {
        // Check it's a symlink
        const attrs = try self.getAttrs(inode_id);
        if (attrs.item_type != @intFromEnum(ItemType.symlink)) {
            return error.InvalidArgument;
        }

        return self.database.readSymlink(inode_id, buf);
    }

    pub fn readDir(self: *Filesystem, dir_inode_id: u64) Error![]DirEntry {
        // Check it's a directory
        const attrs = try self.getAttrs(dir_inode_id);
        if (attrs.item_type != @intFromEnum(ItemType.directory)) {
            return error.NotDirectory;
        }

        return self.database.readDir(dir_inode_id, self.allocator);
    }

    pub fn freeDirEntries(self: *Filesystem, entries: []DirEntry) void {
        for (entries) |entry| {
            self.allocator.free(entry.name);
        }
        self.allocator.free(entries);
    }

    pub fn sync(self: *Filesystem) Error!void {
        return self.database.sync();
    }
};
