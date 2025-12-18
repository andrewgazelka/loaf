const std = @import("std");
const fs = @import("fs.zig");
const err = @import("error.zig");
const overlay_fs = @import("overlay_fs.zig");

pub const Filesystem = fs.Filesystem;
pub const OverlayFs = overlay_fs.OverlayFs;
pub const Attrs = fs.Attrs;
pub const ItemType = fs.ItemType;
pub const DirEntry = fs.DirEntry;
pub const Error = fs.Error;
pub const FfiError = err.FfiError;
pub const toFfi = err.toFfi;

// Use C allocator for FFI stability
const allocator = std.heap.c_allocator;

/// Union of filesystem types for FFI
pub const FsHandle = union(enum) {
    standalone: *Filesystem,
    overlay: *OverlayFs,

    pub fn getRootId(self: FsHandle) u64 {
        return switch (self) {
            .standalone => |f| f.getRootId(),
            .overlay => |o| o.getRootId(),
        };
    }

    pub fn getAttrs(self: FsHandle, inode_id: u64) Error!Attrs {
        return switch (self) {
            .standalone => |f| f.getAttrs(inode_id),
            .overlay => |o| o.getAttrs(inode_id) catch return error.NotFound,
        };
    }

    pub fn lookup(self: FsHandle, parent_id: u64, name: []const u8) Error!u64 {
        return switch (self) {
            .standalone => |f| f.lookup(parent_id, name),
            .overlay => |o| o.lookup(parent_id, name) catch return error.NotFound,
        };
    }

    pub fn create(self: FsHandle, parent_id: u64, name: []const u8, item_type: ItemType, mode: u32) Error!u64 {
        return switch (self) {
            .standalone => |f| f.create(parent_id, name, item_type, mode),
            .overlay => |o| o.create(parent_id, name, item_type, mode) catch return error.IoError,
        };
    }

    pub fn createSymlink(self: FsHandle, parent_id: u64, name: []const u8, target: []const u8) Error!u64 {
        return switch (self) {
            .standalone => |f| f.createSymlink(parent_id, name, target),
            .overlay => |o| o.createSymlink(parent_id, name, target) catch return error.IoError,
        };
    }

    pub fn remove(self: FsHandle, parent_id: u64, inode_id: u64) Error!void {
        return switch (self) {
            .standalone => |f| f.remove(parent_id, inode_id),
            .overlay => |o| o.remove(parent_id, inode_id) catch return error.IoError,
        };
    }

    pub fn rename(self: FsHandle, src_parent_id: u64, src_inode_id: u64, dst_parent_id: u64, dst_name: []const u8) Error!?u64 {
        return switch (self) {
            .standalone => |f| f.rename(src_parent_id, src_inode_id, dst_parent_id, dst_name),
            .overlay => |o| o.rename(src_parent_id, src_inode_id, dst_parent_id, dst_name) catch return error.IoError,
        };
    }

    pub fn read(self: FsHandle, inode_id: u64, offset: i64, buf: []u8) Error!usize {
        return switch (self) {
            .standalone => |f| f.read(inode_id, offset, buf),
            .overlay => |o| o.read(inode_id, offset, buf) catch return error.NotFound,
        };
    }

    pub fn write(self: FsHandle, inode_id: u64, offset: i64, data: []const u8) Error!usize {
        return switch (self) {
            .standalone => |f| f.write(inode_id, offset, data),
            .overlay => |o| o.write(inode_id, offset, data) catch return error.IoError,
        };
    }

    pub fn readSymlink(self: FsHandle, inode_id: u64, buf: []u8) Error!usize {
        return switch (self) {
            .standalone => |f| f.readSymlink(inode_id, buf),
            .overlay => |o| o.readSymlink(inode_id, buf) catch return error.NotFound,
        };
    }

    pub fn readDir(self: FsHandle) []DirEntry {
        _ = self;
        // Not used directly - see DirIterator
        return &[_]DirEntry{};
    }

    pub fn sync(self: FsHandle) Error!void {
        return switch (self) {
            .standalone => |f| f.sync(),
            .overlay => |o| o.sync() catch {},
        };
    }

    pub fn deinit(self: FsHandle) void {
        switch (self) {
            .standalone => |f| f.deinit(),
            .overlay => |o| o.deinit(),
        }
    }
};

// ============================================================================
// FFI Helpers
// ============================================================================

fn handleError(e: Error) c_int {
    return @intFromEnum(toFfi(e));
}

fn ok() c_int {
    return @intFromEnum(FfiError.ok);
}

/// FFI wrapper that stores the FsHandle
const FsWrapper = struct {
    handle: FsHandle,
};

// ============================================================================
// Lifecycle
// ============================================================================

export fn loaf_open(db_path: [*:0]const u8, out_fs: *?*FsWrapper) c_int {
    const path = std.mem.span(db_path);

    const path_z = allocator.dupeZ(u8, path) catch {
        out_fs.* = null;
        return handleError(error.OutOfMemory);
    };
    defer allocator.free(path_z);

    const filesystem = Filesystem.init(allocator, path_z) catch |e| {
        out_fs.* = null;
        return handleError(e);
    };

    const wrapper = allocator.create(FsWrapper) catch {
        filesystem.deinit();
        out_fs.* = null;
        return handleError(error.OutOfMemory);
    };
    wrapper.* = .{ .handle = .{ .standalone = filesystem } };

    out_fs.* = wrapper;
    return ok();
}

export fn loaf_overlay_open(db_path: [*:0]const u8, base_path: [*:0]const u8, out_fs: *?*FsWrapper) c_int {
    const db = std.mem.span(db_path);
    const base = std.mem.span(base_path);

    const db_z = allocator.dupeZ(u8, db) catch {
        out_fs.* = null;
        return handleError(error.OutOfMemory);
    };
    defer allocator.free(db_z);

    const overlay = OverlayFs.init(allocator, db_z, base) catch {
        out_fs.* = null;
        return handleError(error.IoError);
    };

    const wrapper = allocator.create(FsWrapper) catch {
        overlay.deinit();
        out_fs.* = null;
        return handleError(error.OutOfMemory);
    };
    wrapper.* = .{ .handle = .{ .overlay = overlay } };

    out_fs.* = wrapper;
    return ok();
}

const db_mod = @import("db.zig");

/// Open overlay, auto-reading base_path from the database.
/// The .loaf file must have been created with `loaf init` which stores base_path.
export fn loaf_overlay_open_auto(db_path: [*:0]const u8, out_fs: *?*FsWrapper) c_int {
    const db_span = std.mem.span(db_path);

    const db_z = allocator.dupeZ(u8, db_span) catch {
        out_fs.* = null;
        return handleError(error.OutOfMemory);
    };

    // First, open database to read base_path
    var temp_db = db_mod.Database.init(.{ .path = db_z, .allocator = allocator }) catch {
        allocator.free(db_z);
        out_fs.* = null;
        return handleError(error.IoError);
    };

    const base_path = temp_db.getBasePath() catch {
        temp_db.deinit();
        allocator.free(db_z);
        out_fs.* = null;
        return handleError(error.NotFound);
    };
    defer allocator.free(base_path);

    // Close temp database
    temp_db.deinit();

    // Now open overlay with the base_path we found
    const overlay = OverlayFs.init(allocator, db_z, base_path) catch {
        allocator.free(db_z);
        out_fs.* = null;
        return handleError(error.IoError);
    };

    allocator.free(db_z);

    const wrapper = allocator.create(FsWrapper) catch {
        overlay.deinit();
        out_fs.* = null;
        return handleError(error.OutOfMemory);
    };
    wrapper.* = .{ .handle = .{ .overlay = overlay } };

    out_fs.* = wrapper;
    return ok();
}

export fn loaf_close(wrapper: *FsWrapper) void {
    wrapper.handle.deinit();
    allocator.destroy(wrapper);
}

// ============================================================================
// Navigation
// ============================================================================

export fn loaf_get_root_id(wrapper: *FsWrapper) u64 {
    return wrapper.handle.getRootId();
}

export fn loaf_get_attrs(wrapper: *FsWrapper, inode_id: u64, out_attrs: *Attrs) c_int {
    const attrs = wrapper.handle.getAttrs(inode_id) catch |e| return handleError(e);
    out_attrs.* = attrs;
    return ok();
}

export fn loaf_lookup(
    wrapper: *FsWrapper,
    parent_id: u64,
    name: [*]const u8,
    name_len: usize,
    out_inode_id: *u64,
) c_int {
    const inode_id = wrapper.handle.lookup(parent_id, name[0..name_len]) catch |e| return handleError(e);
    out_inode_id.* = inode_id;
    return ok();
}

// ============================================================================
// CRUD
// ============================================================================

export fn loaf_create(
    wrapper: *FsWrapper,
    parent_id: u64,
    name: [*]const u8,
    name_len: usize,
    item_type: u8,
    mode: u32,
    out_inode_id: *u64,
) c_int {
    const inode_id = wrapper.handle.create(
        parent_id,
        name[0..name_len],
        @enumFromInt(item_type),
        mode,
    ) catch |e| return handleError(e);
    out_inode_id.* = inode_id;
    return ok();
}

export fn loaf_create_symlink(
    wrapper: *FsWrapper,
    parent_id: u64,
    name: [*]const u8,
    name_len: usize,
    target: [*]const u8,
    target_len: usize,
    out_inode_id: *u64,
) c_int {
    const inode_id = wrapper.handle.createSymlink(
        parent_id,
        name[0..name_len],
        target[0..target_len],
    ) catch |e| return handleError(e);
    out_inode_id.* = inode_id;
    return ok();
}

export fn loaf_remove(wrapper: *FsWrapper, parent_id: u64, inode_id: u64) c_int {
    wrapper.handle.remove(parent_id, inode_id) catch |e| return handleError(e);
    return ok();
}

export fn loaf_rename(
    wrapper: *FsWrapper,
    src_parent_id: u64,
    src_inode_id: u64,
    dst_parent_id: u64,
    dst_name: [*]const u8,
    dst_name_len: usize,
    out_replaced_id: *u64,
) c_int {
    const replaced_id = wrapper.handle.rename(
        src_parent_id,
        src_inode_id,
        dst_parent_id,
        dst_name[0..dst_name_len],
    ) catch |e| return handleError(e);
    out_replaced_id.* = replaced_id orelse 0;
    return ok();
}

// ============================================================================
// I/O
// ============================================================================

export fn loaf_read(
    wrapper: *FsWrapper,
    inode_id: u64,
    offset: i64,
    buf: [*]u8,
    buf_len: usize,
    out_bytes_read: *usize,
) c_int {
    const bytes_read = wrapper.handle.read(inode_id, offset, buf[0..buf_len]) catch |e| return handleError(e);
    out_bytes_read.* = bytes_read;
    return ok();
}

export fn loaf_write(
    wrapper: *FsWrapper,
    inode_id: u64,
    offset: i64,
    data: [*]const u8,
    data_len: usize,
    out_bytes_written: *usize,
) c_int {
    const bytes_written = wrapper.handle.write(inode_id, offset, data[0..data_len]) catch |e| return handleError(e);
    out_bytes_written.* = bytes_written;
    return ok();
}

export fn loaf_read_symlink(
    wrapper: *FsWrapper,
    inode_id: u64,
    buf: [*]u8,
    buf_len: usize,
    out_len: *usize,
) c_int {
    const len = wrapper.handle.readSymlink(inode_id, buf[0..buf_len]) catch |e| return handleError(e);
    out_len.* = len;
    return ok();
}

// ============================================================================
// Directory enumeration
// ============================================================================

pub const DirIterator = struct {
    entries: []DirEntry,
    index: usize,
    handle: FsHandle,
};

export fn loaf_readdir_begin(
    wrapper: *FsWrapper,
    dir_inode_id: u64,
    out_iter: *?*DirIterator,
) c_int {
    const entries = switch (wrapper.handle) {
        .standalone => |f| f.readDir(dir_inode_id) catch |e| {
            out_iter.* = null;
            return handleError(e);
        },
        .overlay => |o| o.readDir(dir_inode_id) catch {
            out_iter.* = null;
            return handleError(error.NotFound);
        },
    };

    const iter = allocator.create(DirIterator) catch {
        switch (wrapper.handle) {
            .standalone => |f| f.freeDirEntries(entries),
            .overlay => |o| o.freeDirEntries(entries),
        }
        out_iter.* = null;
        return handleError(error.OutOfMemory);
    };

    iter.* = .{
        .entries = entries,
        .index = 0,
        .handle = wrapper.handle,
    };

    out_iter.* = iter;
    return ok();
}

export fn loaf_readdir_next(
    iter: *DirIterator,
    out_inode_id: *u64,
    out_name: *[*]const u8,
    out_name_len: *usize,
    out_item_type: *u8,
) c_int {
    if (iter.index >= iter.entries.len) {
        return handleError(error.NotFound);
    }

    const entry = iter.entries[iter.index];
    out_inode_id.* = entry.inode_id;
    out_name.* = entry.name.ptr;
    out_name_len.* = entry.name.len;
    out_item_type.* = @intFromEnum(entry.item_type);

    iter.index += 1;
    return ok();
}

export fn loaf_readdir_end(iter: *DirIterator) void {
    switch (iter.handle) {
        .standalone => |f| f.freeDirEntries(iter.entries),
        .overlay => |o| o.freeDirEntries(iter.entries),
    }
    allocator.destroy(iter);
}

// ============================================================================
// Sync
// ============================================================================

export fn loaf_sync(wrapper: *FsWrapper) c_int {
    wrapper.handle.sync() catch |e| return handleError(e);
    return ok();
}

// ============================================================================
// Tests
// ============================================================================

fn tempPath(comptime name: []const u8) [:0]const u8 {
    return "/tmp/loaf_test_" ++ name ++ ".db";
}

fn tempPathWal(comptime name: []const u8) [:0]const u8 {
    return "/tmp/loaf_test_" ++ name ++ ".db-wal";
}

fn tempPathShm(comptime name: []const u8) [:0]const u8 {
    return "/tmp/loaf_test_" ++ name ++ ".db-shm";
}

fn cleanupTempDb(comptime name: []const u8) void {
    std.fs.cwd().deleteFile(tempPath(name)) catch {};
    std.fs.cwd().deleteFile(tempPathWal(name)) catch {};
    std.fs.cwd().deleteFile(tempPathShm(name)) catch {};
}

test "basic filesystem operations" {
    const testing = std.testing;
    defer cleanupTempDb("basic");

    var filesystem = try Filesystem.init(testing.allocator, tempPath("basic"));
    defer filesystem.deinit();

    // Root should exist
    const root_id = filesystem.getRootId();
    try testing.expectEqual(@as(u64, 1), root_id);

    // Get root attrs
    const root_attrs = try filesystem.getAttrs(root_id);
    try testing.expectEqual(@as(u8, 1), root_attrs.item_type);

    // Create a file
    const file_id = try filesystem.create(root_id, "test.txt", .file, 0o644);
    try testing.expect(file_id > 1);

    // Lookup the file
    const found_id = try filesystem.lookup(root_id, "test.txt");
    try testing.expectEqual(file_id, found_id);

    // Write to file
    const written = try filesystem.write(file_id, 0, "hello world");
    try testing.expectEqual(@as(usize, 11), written);

    // Read back
    var buf: [100]u8 = undefined;
    const read_len = try filesystem.read(file_id, 0, &buf);
    try testing.expectEqual(@as(usize, 11), read_len);
    try testing.expectEqualSlices(u8, "hello world", buf[0..read_len]);

    // Create subdirectory
    const dir_id = try filesystem.create(root_id, "subdir", .directory, 0o755);
    try testing.expect(dir_id > file_id);

    // Read root directory
    const entries = try filesystem.readDir(root_id);
    defer filesystem.freeDirEntries(entries);
    try testing.expectEqual(@as(usize, 2), entries.len);
}

test "symlink operations" {
    const testing = std.testing;
    defer cleanupTempDb("symlink");

    var filesystem = try Filesystem.init(testing.allocator, tempPath("symlink"));
    defer filesystem.deinit();

    const root_id = filesystem.getRootId();

    // Create a file to link to
    const file_id = try filesystem.create(root_id, "target.txt", .file, 0o644);
    _ = try filesystem.write(file_id, 0, "content");

    // Create symlink
    const link_id = try filesystem.createSymlink(root_id, "link.txt", "target.txt");
    try testing.expect(link_id > file_id);

    // Verify symlink type
    const link_attrs = try filesystem.getAttrs(link_id);
    try testing.expectEqual(@as(u8, 2), link_attrs.item_type); // symlink

    // Read symlink target
    var target_buf: [256]u8 = undefined;
    const target_len = try filesystem.readSymlink(link_id, &target_buf);
    try testing.expectEqualSlices(u8, "target.txt", target_buf[0..target_len]);
}

test "rename operations" {
    const testing = std.testing;
    defer cleanupTempDb("rename");

    var filesystem = try Filesystem.init(testing.allocator, tempPath("rename"));
    defer filesystem.deinit();

    const root_id = filesystem.getRootId();

    // Create a file
    const file_id = try filesystem.create(root_id, "old.txt", .file, 0o644);
    _ = try filesystem.write(file_id, 0, "test data");

    // Rename the file
    const replaced = try filesystem.rename(root_id, file_id, root_id, "new.txt");
    try testing.expectEqual(@as(?u64, null), replaced);

    // Old name should not exist
    try testing.expectError(error.NotFound, filesystem.lookup(root_id, "old.txt"));

    // New name should exist with same inode
    const found_id = try filesystem.lookup(root_id, "new.txt");
    try testing.expectEqual(file_id, found_id);

    // Content should be preserved
    var buf: [100]u8 = undefined;
    const read_len = try filesystem.read(file_id, 0, &buf);
    try testing.expectEqualSlices(u8, "test data", buf[0..read_len]);
}

test "rename with replacement" {
    const testing = std.testing;
    defer cleanupTempDb("rename_replace");

    var filesystem = try Filesystem.init(testing.allocator, tempPath("rename_replace"));
    defer filesystem.deinit();

    const root_id = filesystem.getRootId();

    // Create two files
    const file1_id = try filesystem.create(root_id, "file1.txt", .file, 0o644);
    _ = try filesystem.write(file1_id, 0, "content1");

    const file2_id = try filesystem.create(root_id, "file2.txt", .file, 0o644);
    _ = try filesystem.write(file2_id, 0, "content2");

    // Rename file1 over file2
    const replaced = try filesystem.rename(root_id, file1_id, root_id, "file2.txt");
    try testing.expectEqual(file2_id, replaced.?);

    // file1 name should not exist
    try testing.expectError(error.NotFound, filesystem.lookup(root_id, "file1.txt"));

    // file2 name should point to file1's content
    const found_id = try filesystem.lookup(root_id, "file2.txt");
    try testing.expectEqual(file1_id, found_id);

    var buf: [100]u8 = undefined;
    const read_len = try filesystem.read(found_id, 0, &buf);
    try testing.expectEqualSlices(u8, "content1", buf[0..read_len]);
}

test "remove file" {
    const testing = std.testing;
    defer cleanupTempDb("remove_file");

    var filesystem = try Filesystem.init(testing.allocator, tempPath("remove_file"));
    defer filesystem.deinit();

    const root_id = filesystem.getRootId();

    // Create and remove a file
    const file_id = try filesystem.create(root_id, "delete_me.txt", .file, 0o644);
    try filesystem.remove(root_id, file_id);

    // File should not exist
    try testing.expectError(error.NotFound, filesystem.lookup(root_id, "delete_me.txt"));
    try testing.expectError(error.NotFound, filesystem.getAttrs(file_id));
}

test "remove empty directory" {
    const testing = std.testing;
    defer cleanupTempDb("remove_dir");

    var filesystem = try Filesystem.init(testing.allocator, tempPath("remove_dir"));
    defer filesystem.deinit();

    const root_id = filesystem.getRootId();

    // Create and remove an empty directory
    const dir_id = try filesystem.create(root_id, "empty_dir", .directory, 0o755);
    try filesystem.remove(root_id, dir_id);

    // Directory should not exist
    try testing.expectError(error.NotFound, filesystem.lookup(root_id, "empty_dir"));
}

test "cannot remove non-empty directory" {
    const testing = std.testing;
    defer cleanupTempDb("nonempty");

    var filesystem = try Filesystem.init(testing.allocator, tempPath("nonempty"));
    defer filesystem.deinit();

    const root_id = filesystem.getRootId();

    // Create directory with a file inside
    const dir_id = try filesystem.create(root_id, "nonempty_dir", .directory, 0o755);
    _ = try filesystem.create(dir_id, "file.txt", .file, 0o644);

    // Should fail to remove non-empty directory
    try testing.expectError(error.NotEmpty, filesystem.remove(root_id, dir_id));
}

test "nested directory operations" {
    const testing = std.testing;
    defer cleanupTempDb("nested");

    var filesystem = try Filesystem.init(testing.allocator, tempPath("nested"));
    defer filesystem.deinit();

    const root_id = filesystem.getRootId();

    // Create nested structure: /a/b/c/file.txt
    const dir_a = try filesystem.create(root_id, "a", .directory, 0o755);
    const dir_b = try filesystem.create(dir_a, "b", .directory, 0o755);
    const dir_c = try filesystem.create(dir_b, "c", .directory, 0o755);
    const file_id = try filesystem.create(dir_c, "file.txt", .file, 0o644);

    _ = try filesystem.write(file_id, 0, "nested content");

    // Verify path traversal
    const found_a = try filesystem.lookup(root_id, "a");
    try testing.expectEqual(dir_a, found_a);

    const found_b = try filesystem.lookup(dir_a, "b");
    try testing.expectEqual(dir_b, found_b);

    const found_c = try filesystem.lookup(dir_b, "c");
    try testing.expectEqual(dir_c, found_c);

    const found_file = try filesystem.lookup(dir_c, "file.txt");
    try testing.expectEqual(file_id, found_file);

    // Read content
    var buf: [100]u8 = undefined;
    const read_len = try filesystem.read(file_id, 0, &buf);
    try testing.expectEqualSlices(u8, "nested content", buf[0..read_len]);
}

test "write at offset" {
    const testing = std.testing;
    defer cleanupTempDb("offset");

    var filesystem = try Filesystem.init(testing.allocator, tempPath("offset"));
    defer filesystem.deinit();

    const root_id = filesystem.getRootId();
    const file_id = try filesystem.create(root_id, "offset.txt", .file, 0o644);

    // Write "hello" at start
    _ = try filesystem.write(file_id, 0, "hello");

    // Write "world" at offset 6 (leaving a gap)
    _ = try filesystem.write(file_id, 6, "world");

    // Read full content
    var buf: [100]u8 = undefined;
    const read_len = try filesystem.read(file_id, 0, &buf);
    try testing.expectEqual(@as(usize, 11), read_len);
    try testing.expectEqualSlices(u8, "hello\x00world", buf[0..read_len]);

    // Read at offset
    const partial_len = try filesystem.read(file_id, 6, &buf);
    try testing.expectEqualSlices(u8, "world", buf[0..partial_len]);
}

test "overwrite middle of file" {
    const testing = std.testing;
    defer cleanupTempDb("overwrite");

    var filesystem = try Filesystem.init(testing.allocator, tempPath("overwrite"));
    defer filesystem.deinit();

    const root_id = filesystem.getRootId();
    const file_id = try filesystem.create(root_id, "overwrite.txt", .file, 0o644);

    // Write initial content
    _ = try filesystem.write(file_id, 0, "hello world");

    // Overwrite "world" with "there"
    _ = try filesystem.write(file_id, 6, "there");

    // Read back
    var buf: [100]u8 = undefined;
    const read_len = try filesystem.read(file_id, 0, &buf);
    try testing.expectEqualSlices(u8, "hello there", buf[0..read_len]);
}

test "duplicate name creates error" {
    const testing = std.testing;
    defer cleanupTempDb("duplicate");

    var filesystem = try Filesystem.init(testing.allocator, tempPath("duplicate"));
    defer filesystem.deinit();

    const root_id = filesystem.getRootId();

    // Create a file
    _ = try filesystem.create(root_id, "duplicate.txt", .file, 0o644);

    // Try to create another with same name
    try testing.expectError(error.AlreadyExists, filesystem.create(root_id, "duplicate.txt", .file, 0o644));

    // Same for directories
    _ = try filesystem.create(root_id, "dup_dir", .directory, 0o755);
    try testing.expectError(error.AlreadyExists, filesystem.create(root_id, "dup_dir", .directory, 0o755));
}

test "sync does not error" {
    const testing = std.testing;
    defer cleanupTempDb("sync");

    var filesystem = try Filesystem.init(testing.allocator, tempPath("sync"));
    defer filesystem.deinit();

    // Sync should succeed on empty database
    try filesystem.sync();

    // Create some content and sync again
    const root_id = filesystem.getRootId();
    const file_id = try filesystem.create(root_id, "sync_test.txt", .file, 0o644);
    _ = try filesystem.write(file_id, 0, "data to sync");

    try filesystem.sync();
}
