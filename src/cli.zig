const std = @import("std");
const fs_mod = @import("fs.zig");
const db_mod = @import("db.zig");
const overlay_mod = @import("overlay.zig");

const Filesystem = fs_mod.Filesystem;
const Overlay = overlay_mod.Overlay;
const ItemType = db_mod.ItemType;
const DirEntry = db_mod.DirEntry;

// =============================================================================
// Types
// =============================================================================

const Command = enum {
    // Overlay commands
    init,
    diff,
    accept,
    reject,
    status,
    run,
    mount,
    unmount,
    // Inspection commands
    ls,
    cat,
    tree,
    info,
    help,
};

const CliError = error{
    InvalidCommand,
    MissingArgument,
    PathNotFound,
    InvalidPath,
};

// =============================================================================
// Usage / Help
// =============================================================================

fn printUsage(writer: anytype) !void {
    try writer.writeAll(
        \\loaf - Overlay filesystem for macOS
        \\
        \\SANDBOX COMMANDS:
        \\  loaf run <cmd> [args...]           Run command in FSKit overlay sandbox
        \\                                     Changes are captured and can be accepted/rejected
        \\
        \\OVERLAY COMMANDS:
        \\  loaf init <path> [overlay.loaf]    Create overlay on directory
        \\  loaf diff [overlay.loaf]           Show pending changes
        \\  loaf accept [overlay.loaf]         Apply changes to real filesystem
        \\  loaf reject [overlay.loaf]         Discard all changes
        \\  loaf status                        Show active overlays
        \\
        \\FSKIT MOUNT COMMANDS:
        \\  loaf mount <overlay.loaf> <mount>  Mount .loaf file via FSKit
        \\  loaf unmount <mount>               Unmount FSKit filesystem
        \\
        \\INSPECTION COMMANDS:
        \\  loaf ls [db.loaf] [path]           List directory contents
        \\  loaf cat [db.loaf] <path>          Read file contents
        \\  loaf tree [db.loaf]                Show full directory tree
        \\  loaf info [db.loaf]                Show database stats
        \\
        \\EXAMPLES:
        \\  # Run Claude in sandboxed overlay (FSKit)
        \\  loaf run claude --dangerously-skip-permissions
        \\
        \\  # Create persistent overlay on a project
        \\  loaf init ~/Projects/myapp
        \\
        \\  # Review and apply changes
        \\  loaf diff
        \\  loaf accept   # Apply to real files
        \\  loaf reject   # Discard changes
        \\
        \\  # Manual FSKit mount
        \\  loaf mount overlay.loaf /tmp/mnt
        \\
    );
}

fn parseCommand(arg: []const u8) ?Command {
    const map = std.StaticStringMap(Command).initComptime(.{
        // Sandbox
        .{ "run", .run },
        // Overlay
        .{ "init", .init },
        .{ "diff", .diff },
        .{ "accept", .accept },
        .{ "reject", .reject },
        .{ "status", .status },
        // FSKit mount
        .{ "mount", .mount },
        .{ "unmount", .unmount },
        // Inspection
        .{ "ls", .ls },
        .{ "cat", .cat },
        .{ "tree", .tree },
        .{ "info", .info },
        .{ "help", .help },
        .{ "--help", .help },
        .{ "-h", .help },
    });
    return map.get(arg);
}

// =============================================================================
// Overlay Commands
// =============================================================================

fn findDefaultOverlay(allocator: std.mem.Allocator) ?[:0]const u8 {
    // Look for .loaf file in current directory or parent directories
    var path_buf: [std.fs.max_path_bytes]u8 = undefined;
    var cwd: []const u8 = std.fs.cwd().realpath(".", &path_buf) catch return null;

    while (true) {
        const loaf_path = std.fmt.allocPrintSentinel(allocator, "{s}/.loaf", .{cwd}, 0) catch return null;

        if (std.fs.cwd().access(loaf_path, .{})) |_| {
            return loaf_path;
        } else |_| {
            allocator.free(loaf_path);
        }

        // Go to parent
        const parent = std.fs.path.dirname(cwd) orelse break;
        if (std.mem.eql(u8, parent, cwd)) break;
        cwd = parent;
    }

    return null;
}

fn cmdInit(allocator: std.mem.Allocator, base_path: []const u8, overlay_path_opt: ?[]const u8, writer: anytype) !void {
    // Resolve base path to absolute
    var path_buf: [std.fs.max_path_bytes]u8 = undefined;
    const abs_base = std.fs.cwd().realpath(base_path, &path_buf) catch {
        try writer.print("Error: path not found: {s}\n", .{base_path});
        return;
    };

    // Default overlay path is .loaf in the target directory
    const overlay_path = if (overlay_path_opt) |p|
        try allocator.dupeZ(u8, p)
    else
        try std.fmt.allocPrintSentinel(allocator, "{s}/.loaf", .{abs_base}, 0);
    defer allocator.free(overlay_path);

    // Check if overlay already exists
    if (std.fs.cwd().access(overlay_path, .{})) |_| {
        try writer.print("Overlay already exists: {s}\n", .{overlay_path});
        return;
    } else |_| {}

    // Create overlay
    const overlay = Overlay.init(allocator, overlay_path, abs_base) catch |err| {
        try writer.print("Error creating overlay: {s}\n", .{@errorName(err)});
        return;
    };
    defer overlay.deinit();

    try writer.print("Created overlay: {s}\n", .{overlay_path});
    try writer.print("Base path: {s}\n", .{abs_base});
    try writer.print("\nChanges will be captured in the overlay.\n", .{});
    try writer.print("Use 'loaf diff' to review, 'loaf accept' or 'loaf reject' to finalize.\n", .{});
}

fn cmdDiff(allocator: std.mem.Allocator, overlay_path: [:0]const u8, writer: anytype) !void {
    const overlay = Overlay.init(allocator, overlay_path, "") catch |err| {
        // Try to get base path from existing overlay
        var db = db_mod.Database.init(.{ .path = overlay_path, .allocator = allocator }) catch {
            try writer.print("Error opening overlay: {s}\n", .{@errorName(err)});
            return;
        };
        const base_path = db.getBasePath() catch {
            db.deinit();
            try writer.print("Error: overlay has no base path\n", .{});
            return;
        };
        defer allocator.free(base_path);
        db.deinit();

        const o = Overlay.init(allocator, overlay_path, base_path) catch {
            try writer.print("Error opening overlay\n", .{});
            return;
        };
        defer o.deinit();

        const changes = o.getChanges() catch {
            try writer.print("Error getting changes\n", .{});
            return;
        };
        defer o.freeChanges(changes);

        if (changes.len == 0) {
            try writer.print("No changes.\n", .{});
            return;
        }

        try writer.print("{d} changed file(s):\n\n", .{changes.len});
        for (changes) |change| {
            const prefix: []const u8 = switch (change.change_type) {
                .added => "+ ",
                .modified => "~ ",
                .deleted => "- ",
            };
            const color: []const u8 = switch (change.change_type) {
                .added => "\x1b[32m",
                .modified => "\x1b[33m",
                .deleted => "\x1b[31m",
            };
            try writer.print("{s}{s}{s}\x1b[0m\n", .{ color, prefix, change.path });
        }
        return;
    };
    defer overlay.deinit();

    const changes = overlay.getChanges() catch {
        try writer.print("Error getting changes\n", .{});
        return;
    };
    defer overlay.freeChanges(changes);

    if (changes.len == 0) {
        try writer.print("No changes.\n", .{});
        return;
    }

    try writer.print("{d} changed file(s):\n\n", .{changes.len});
    for (changes) |change| {
        const prefix: []const u8 = switch (change.change_type) {
            .added => "+ ",
            .modified => "~ ",
            .deleted => "- ",
        };
        const color: []const u8 = switch (change.change_type) {
            .added => "\x1b[32m",
            .modified => "\x1b[33m",
            .deleted => "\x1b[31m",
        };
        try writer.print("{s}{s}{s}\x1b[0m\n", .{ color, prefix, change.path });
    }
}

fn cmdAccept(allocator: std.mem.Allocator, overlay_path: [:0]const u8, writer: anytype) !void {
    var db = db_mod.Database.init(.{ .path = overlay_path, .allocator = allocator }) catch {
        try writer.print("Error opening overlay\n", .{});
        return;
    };
    const base_path = db.getBasePath() catch {
        db.deinit();
        try writer.print("Error: overlay has no base path\n", .{});
        return;
    };
    defer allocator.free(base_path);
    db.deinit();

    const overlay = Overlay.init(allocator, overlay_path, base_path) catch {
        try writer.print("Error opening overlay\n", .{});
        return;
    };
    defer overlay.deinit();

    const changes = overlay.getChanges() catch {
        try writer.print("Error getting changes\n", .{});
        return;
    };
    const change_count = changes.len;
    overlay.freeChanges(changes);

    if (change_count == 0) {
        try writer.print("No changes to apply.\n", .{});
        return;
    }

    overlay.accept() catch {
        try writer.print("Error applying changes\n", .{});
        return;
    };

    try writer.print("Applied {d} change(s) to {s}\n", .{ change_count, base_path });
}

fn cmdReject(allocator: std.mem.Allocator, overlay_path: [:0]const u8, writer: anytype) !void {
    var db = db_mod.Database.init(.{ .path = overlay_path, .allocator = allocator }) catch {
        try writer.print("Error opening overlay\n", .{});
        return;
    };
    const base_path = db.getBasePath() catch {
        db.deinit();
        try writer.print("Error: overlay has no base path\n", .{});
        return;
    };
    defer allocator.free(base_path);
    db.deinit();

    const overlay = Overlay.init(allocator, overlay_path, base_path) catch {
        try writer.print("Error opening overlay\n", .{});
        return;
    };
    defer overlay.deinit();

    overlay.reject() catch {
        try writer.print("Error rejecting changes\n", .{});
        return;
    };

    try writer.print("Discarded all changes.\n", .{});
}

fn cmdStatus(allocator: std.mem.Allocator, writer: anytype) !void {
    _ = allocator;
    // For now, just look for .loaf in current directory
    if (std.fs.cwd().access(".loaf", .{})) |_| {
        try writer.print("Active overlay: .loaf\n", .{});
    } else |_| {
        try writer.print("No active overlay in current directory.\n", .{});
        try writer.print("Use 'loaf init <path>' to create one.\n", .{});
    }
}

// =============================================================================
// FSKit Mount Commands
// =============================================================================

fn cmdMount(allocator: std.mem.Allocator, loaf_path: []const u8, mount_point: []const u8, stdout: anytype, stderr: anytype) !u8 {
    // Resolve .loaf path to absolute
    var loaf_buf: [std.fs.max_path_bytes]u8 = undefined;
    const abs_loaf = std.fs.cwd().realpath(loaf_path, &loaf_buf) catch {
        try stderr.print("Error: .loaf file not found: {s}\n", .{loaf_path});
        return 1;
    };

    // Create mount point if it doesn't exist
    std.fs.cwd().makePath(mount_point) catch |err| {
        if (err != error.PathAlreadyExists) {
            try stderr.print("Error: cannot create mount point: {s}\n", .{mount_point});
            return 1;
        }
    };

    // Resolve mount point to absolute
    var mount_buf: [std.fs.max_path_bytes]u8 = undefined;
    const abs_mount = std.fs.cwd().realpath(mount_point, &mount_buf) catch {
        try stderr.print("Error: mount point not found: {s}\n", .{mount_point});
        return 1;
    };

    // Mount via FSKit: mount -F -t loaf /path/to.loaf /mount/point
    try stdout.print("Mounting {s} at {s}...\n", .{ abs_loaf, abs_mount });
    try stdout.flush();

    const mount_args = [_][]const u8{ "mount", "-F", "-t", "loaf", abs_loaf, abs_mount };
    var mount_proc = std.process.Child.init(&mount_args, allocator);
    mount_proc.stderr_behavior = .Inherit;
    mount_proc.stdout_behavior = .Inherit;
    mount_proc.spawn() catch {
        try stderr.writeAll("Error: failed to spawn mount command\n");
        return 1;
    };
    const result = mount_proc.wait() catch {
        try stderr.writeAll("Error: mount command failed\n");
        return 1;
    };

    return switch (result) {
        .Exited => |code| {
            if (code == 0) {
                try stdout.print("Mounted successfully.\n", .{});
            }
            return code;
        },
        else => 1,
    };
}

fn cmdUnmount(allocator: std.mem.Allocator, mount_point: []const u8, stdout: anytype, stderr: anytype) !u8 {
    // Resolve mount point to absolute
    var mount_buf: [std.fs.max_path_bytes]u8 = undefined;
    const abs_mount = std.fs.cwd().realpath(mount_point, &mount_buf) catch {
        try stderr.print("Error: mount point not found: {s}\n", .{mount_point});
        return 1;
    };

    try stdout.print("Unmounting {s}...\n", .{abs_mount});
    try stdout.flush();

    const umount_args = [_][]const u8{ "umount", abs_mount };
    var umount_proc = std.process.Child.init(&umount_args, allocator);
    umount_proc.stderr_behavior = .Inherit;
    umount_proc.stdout_behavior = .Inherit;
    umount_proc.spawn() catch {
        try stderr.writeAll("Error: failed to spawn umount command\n");
        return 1;
    };
    const result = umount_proc.wait() catch {
        try stderr.writeAll("Error: umount command failed\n");
        return 1;
    };

    return switch (result) {
        .Exited => |code| {
            if (code == 0) {
                try stdout.print("Unmounted successfully.\n", .{});
            }
            return code;
        },
        else => 1,
    };
}

// =============================================================================
// Run Command (FSKit overlay sandbox execution)
// =============================================================================

fn cmdRun(allocator: std.mem.Allocator, cmd_args: []const []const u8, stdout: anytype, stderr: anytype) !u8 {
    // Get current working directory
    var cwd_buf: [std.fs.max_path_bytes]u8 = undefined;
    const cwd = std.fs.cwd().realpath(".", &cwd_buf) catch {
        try stderr.writeAll("Error: cannot determine current directory\n");
        return 1;
    };

    // Create temp paths for overlay and mount point
    const timestamp = @as(u64, @intCast(std.time.timestamp()));
    const overlay_path = std.fmt.allocPrintSentinel(allocator, "/tmp/loaf-sandbox-{d}.loaf", .{timestamp}, 0) catch {
        try stderr.writeAll("Error: out of memory\n");
        return 1;
    };
    defer allocator.free(overlay_path);

    const mount_point = std.fmt.allocPrint(allocator, "/tmp/loaf-mount-{d}", .{timestamp}) catch {
        try stderr.writeAll("Error: out of memory\n");
        return 1;
    };
    defer allocator.free(mount_point);

    // Create mount point directory
    std.fs.cwd().makePath(mount_point) catch {
        try stderr.print("Error: cannot create mount point: {s}\n", .{mount_point});
        return 1;
    };

    // Create the overlay database pointing to cwd
    try stdout.print("\x1b[90m● Creating FSKit overlay...\x1b[0m\n", .{});
    try stdout.flush();

    const overlay = Overlay.init(allocator, overlay_path, cwd) catch |err| {
        try stderr.print("Error creating overlay: {s}\n", .{@errorName(err)});
        cleanupSandbox(mount_point);
        return 1;
    };
    overlay.deinit();

    // Mount via FSKit
    try stdout.print("\x1b[90m● Mounting overlay at {s}...\x1b[0m\n", .{mount_point});
    try stdout.flush();

    const mount_args = [_][]const u8{ "mount", "-F", "-t", "loaf", overlay_path, mount_point };
    var mount_proc = std.process.Child.init(&mount_args, allocator);
    mount_proc.stderr_behavior = .Pipe;
    mount_proc.spawn() catch {
        try stderr.writeAll("Error: failed to spawn mount command\n");
        cleanupOverlayAndMount(overlay_path, mount_point);
        return 1;
    };
    const mount_result = mount_proc.wait() catch {
        try stderr.writeAll("Error: mount command failed\n");
        cleanupOverlayAndMount(overlay_path, mount_point);
        return 1;
    };

    const mount_code = switch (mount_result) {
        .Exited => |code| code,
        else => 1,
    };

    if (mount_code != 0) {
        // FSKit mount failed - offer to open the app
        try stderr.writeAll("\x1b[31mError: FSKit extension not enabled.\x1b[0m\n\n");
        try stderr.writeAll("The Loaf filesystem extension needs to be enabled.\n");
        try stderr.writeAll("Open Loaf.app to register it? [Y/n]: ");
        try stderr.flush();

        const response = readUserInput() catch 'y';
        if (response != 'n' and response != 'N') {
            // Try to open Loaf.app
            const open_args = [_][]const u8{ "open", "-a", "Loaf" };
            var open_proc = std.process.Child.init(&open_args, allocator);
            open_proc.spawn() catch {};
            _ = open_proc.wait() catch {};

            try stderr.writeAll("\nAfter the app opens:\n");
            try stderr.writeAll("  1. System Settings > General > Login Items & Extensions\n");
            try stderr.writeAll("  2. Click 'File System Extensions'\n");
            try stderr.writeAll("  3. Enable 'Loaf'\n");
            try stderr.writeAll("  4. Run this command again\n\n");
        }
        cleanupOverlayAndMount(overlay_path, mount_point);
        return 1;
    }

    // Run the command in mounted overlay
    try stdout.print("\x1b[90m● Running command in overlay...\x1b[0m\n\n", .{});
    try stdout.flush();

    const exit_code = runInSandbox(allocator, cmd_args, mount_point);

    try stdout.print("\n", .{});

    // Unmount before computing diff
    const umount_args = [_][]const u8{ "umount", mount_point };
    var umount_proc = std.process.Child.init(&umount_args, allocator);
    umount_proc.spawn() catch {};
    _ = umount_proc.wait() catch {};

    // Get changes from overlay
    const overlay2 = Overlay.init(allocator, overlay_path, cwd) catch {
        try stderr.writeAll("Error: failed to read overlay changes\n");
        cleanupOverlayAndMount(overlay_path, mount_point);
        return 1;
    };
    defer overlay2.deinit();

    const changes = overlay2.getChanges() catch {
        try stderr.writeAll("Error: failed to get changes\n");
        cleanupOverlayAndMount(overlay_path, mount_point);
        return 1;
    };
    defer overlay2.freeChanges(changes);

    if (changes.len == 0) {
        try stdout.print("\x1b[90m● No changes.\x1b[0m\n", .{});
        cleanupOverlayAndMount(overlay_path, mount_point);
        return exit_code;
    }

    // Show changes summary
    try stdout.print("\x1b[33mloaf:\x1b[0m {d} changed file(s)\n", .{changes.len});
    for (changes) |change| {
        const prefix: []const u8 = switch (change.change_type) {
            .added => "\x1b[32m+ ",
            .modified => "\x1b[33m~ ",
            .deleted => "\x1b[31m- ",
        };
        try stdout.print("  {s}{s}\x1b[0m\n", .{ prefix, change.path });
    }

    // Prompt for accept/reject
    try stdout.print("\n\x1b[1mApply these changes to {s}?\x1b[0m [y/N]: ", .{cwd});
    try stdout.flush();

    // Read user input
    const response = readUserInput() catch 'n';

    switch (response) {
        'y', 'Y' => {
            // Apply changes from overlay to real filesystem
            overlay2.accept() catch {
                try stderr.writeAll("Error: failed to apply changes\n");
                cleanupOverlayAndMount(overlay_path, mount_point);
                return 1;
            };
            try stdout.print("\x1b[32m✓ Applied changes.\x1b[0m\n", .{});
        },
        else => {
            try stdout.print("\x1b[90m● Discarded changes.\x1b[0m\n", .{});
        },
    }

    // Cleanup
    cleanupOverlayAndMount(overlay_path, mount_point);
    return exit_code;
}

fn cleanupOverlayAndMount(overlay_path: []const u8, mount_point: []const u8) void {
    // Remove overlay database
    std.fs.cwd().deleteFile(overlay_path) catch {};
    // Remove mount point directory
    std.fs.cwd().deleteTree(mount_point) catch {};
}

const Change = struct {
    path: []const u8,
    change_type: ChangeType,
};

const ChangeType = enum {
    added,
    modified,
    deleted,
};

fn cloneDirectory(allocator: std.mem.Allocator, src: []const u8, dst: []const u8) u8 {
    // Use macOS native cp for APFS clone support (-c flag)
    // /bin/cp is macOS's built-in cp which supports -c for APFS clones
    // Use "src/." to copy contents only, not the directory itself
    const src_contents = std.fmt.allocPrint(allocator, "{s}/.", .{src}) catch return 1;
    defer allocator.free(src_contents);

    // Try APFS clone first
    const clone_args = [_][]const u8{ "/bin/cp", "-cR", src_contents, dst };
    var clone_proc = std.process.Child.init(&clone_args, allocator);
    clone_proc.stderr_behavior = .Ignore;
    clone_proc.spawn() catch {
        // Fall back to regular copy
        const cp_args = [_][]const u8{ "/bin/cp", "-R", src_contents, dst };
        var cp_proc = std.process.Child.init(&cp_args, allocator);
        cp_proc.spawn() catch return 1;
        const result = cp_proc.wait() catch return 1;
        return switch (result) {
            .Exited => |code| code,
            else => 1,
        };
    };
    const result = clone_proc.wait() catch return 1;

    return switch (result) {
        .Exited => |code| code,
        else => 1,
    };
}

fn runInSandbox(allocator: std.mem.Allocator, cmd_args: []const []const u8, sandbox_dir: []const u8) u8 {
    var child = std.process.Child.init(cmd_args, allocator);
    child.cwd = sandbox_dir;
    child.stdin_behavior = .Inherit;
    child.stdout_behavior = .Inherit;
    child.stderr_behavior = .Inherit;

    child.spawn() catch return 127;
    const result = child.wait() catch return 1;

    return switch (result) {
        .Exited => |code| code,
        .Signal => 128,
        else => 1,
    };
}

fn computeChanges(allocator: std.mem.Allocator, original: []const u8, modified: []const u8) ![]Change {
    var changes: std.ArrayList(Change) = .empty;
    errdefer {
        for (changes.items) |c| allocator.free(c.path);
        changes.deinit(allocator);
    }

    // Walk the modified directory and compare with original
    try walkAndCompare(allocator, "", original, modified, &changes, false);

    // Walk original to find deletions
    try walkAndCompare(allocator, "", modified, original, &changes, true);

    return changes.toOwnedSlice(allocator);
}

fn walkAndCompare(
    allocator: std.mem.Allocator,
    rel_path: []const u8,
    base_dir: []const u8,
    compare_dir: []const u8,
    changes: *std.ArrayList(Change),
    check_deletions: bool,
) !void {
    const compare_path = if (rel_path.len > 0)
        try std.fmt.allocPrint(allocator, "{s}/{s}", .{ compare_dir, rel_path })
    else
        try allocator.dupe(u8, compare_dir);
    defer allocator.free(compare_path);

    var dir = std.fs.cwd().openDir(compare_path, .{ .iterate = true }) catch return;
    defer dir.close();

    var iter = dir.iterate();
    while (try iter.next()) |entry| {
        // Skip .loaf files and hidden files
        if (std.mem.startsWith(u8, entry.name, ".")) continue;

        const child_rel = if (rel_path.len > 0)
            try std.fmt.allocPrint(allocator, "{s}/{s}", .{ rel_path, entry.name })
        else
            try allocator.dupe(u8, entry.name);

        const base_child = try std.fmt.allocPrint(allocator, "{s}/{s}", .{ base_dir, child_rel });
        defer allocator.free(base_child);

        const compare_child = try std.fmt.allocPrint(allocator, "{s}/{s}", .{ compare_dir, child_rel });
        defer allocator.free(compare_child);

        if (check_deletions) {
            // Looking for files in original that don't exist in modified
            if (std.fs.cwd().access(base_child, .{})) |_| {
                // Exists in both - already handled
                allocator.free(child_rel);
            } else |_| {
                // Exists in original but not in modified = deleted
                try changes.append(allocator, .{
                    .path = child_rel,
                    .change_type = .deleted,
                });
            }
        } else {
            // Looking for new or modified files
            if (std.fs.cwd().access(base_child, .{})) |_| {
                // Exists in both - check if modified
                if (entry.kind == .file) {
                    if (try filesAreDifferent(allocator, base_child, compare_child)) {
                        try changes.append(allocator, .{
                            .path = child_rel,
                            .change_type = .modified,
                        });
                    } else {
                        allocator.free(child_rel);
                    }
                } else if (entry.kind == .directory) {
                    allocator.free(child_rel);
                    const nested_rel = if (rel_path.len > 0)
                        try std.fmt.allocPrint(allocator, "{s}/{s}", .{ rel_path, entry.name })
                    else
                        try allocator.dupe(u8, entry.name);
                    defer allocator.free(nested_rel);
                    try walkAndCompare(allocator, nested_rel, base_dir, compare_dir, changes, false);
                } else {
                    allocator.free(child_rel);
                }
            } else |_| {
                // New file
                try changes.append(allocator, .{
                    .path = child_rel,
                    .change_type = .added,
                });
            }
        }
    }
}

fn filesAreDifferent(allocator: std.mem.Allocator, path1: []const u8, path2: []const u8) !bool {
    // Quick check: compare sizes first
    const stat1 = std.fs.cwd().statFile(path1) catch return true;
    const stat2 = std.fs.cwd().statFile(path2) catch return true;

    if (stat1.size != stat2.size) return true;

    // Compare content using hash
    const diff_args = [_][]const u8{ "diff", "-q", path1, path2 };
    var diff_proc = std.process.Child.init(&diff_args, allocator);
    diff_proc.stdout_behavior = .Ignore;
    diff_proc.stderr_behavior = .Ignore;
    diff_proc.spawn() catch return true;
    const result = diff_proc.wait() catch return true;

    return switch (result) {
        .Exited => |code| code != 0,
        else => true,
    };
}

fn readUserInput() !u8 {
    var buf: [16]u8 = undefined;
    const stdin = std.fs.File.stdin();
    const n = stdin.read(&buf) catch return error.ReadError;
    if (n == 0) return 'n';
    return buf[0];
}

fn applyChanges(allocator: std.mem.Allocator, target: []const u8, source: []const u8, changes: []const Change) !void {
    for (changes) |change| {
        const target_path = try std.fmt.allocPrint(allocator, "{s}/{s}", .{ target, change.path });
        defer allocator.free(target_path);

        const source_path = try std.fmt.allocPrint(allocator, "{s}/{s}", .{ source, change.path });
        defer allocator.free(source_path);

        switch (change.change_type) {
            .added, .modified => {
                // Copy from source to target
                const parent = std.fs.path.dirname(target_path) orelse ".";
                std.fs.cwd().makePath(parent) catch {};

                const cp_args = [_][]const u8{ "cp", "-f", source_path, target_path };
                var cp_proc = std.process.Child.init(&cp_args, allocator);
                cp_proc.spawn() catch continue;
                _ = cp_proc.wait() catch continue;
            },
            .deleted => {
                std.fs.cwd().deleteFile(target_path) catch |e| {
                    if (e != error.FileNotFound) {
                        std.fs.cwd().deleteTree(target_path) catch {};
                    }
                };
            },
        }
    }
}

fn showDiff(allocator: std.mem.Allocator, original: []const u8, modified: []const u8, changes: []const Change, writer: anytype) void {
    for (changes) |change| {
        const orig_path = std.fmt.allocPrint(allocator, "{s}/{s}", .{ original, change.path }) catch continue;
        defer allocator.free(orig_path);

        const mod_path = std.fmt.allocPrint(allocator, "{s}/{s}", .{ modified, change.path }) catch continue;
        defer allocator.free(mod_path);

        writer.print("--- {s}\n", .{change.path}) catch continue;

        switch (change.change_type) {
            .added => {
                writer.print("\x1b[32m+++ (new file)\x1b[0m\n", .{}) catch continue;
            },
            .deleted => {
                writer.print("\x1b[31m--- (deleted)\x1b[0m\n", .{}) catch continue;
            },
            .modified => {
                // Run diff
                const diff_args = [_][]const u8{ "diff", "-u", orig_path, mod_path };
                var diff_proc = std.process.Child.init(&diff_args, allocator);
                diff_proc.stdout_behavior = .Inherit;
                diff_proc.stderr_behavior = .Inherit;
                diff_proc.spawn() catch continue;
                _ = diff_proc.wait() catch continue;
            },
        }
        writer.print("\n", .{}) catch continue;
    }
}

fn cleanupSandbox(path: []const u8) void {
    // Use rm -rf since it's faster for large directories
    var rm_proc = std.process.Child.init(&.{ "rm", "-rf", path }, std.heap.page_allocator);
    rm_proc.spawn() catch return;
    _ = rm_proc.wait() catch return;
}

// =============================================================================
// Inspection Commands (existing)
// =============================================================================

fn resolvePath(fs: *Filesystem, path: []const u8) !u64 {
    if (path.len == 0 or std.mem.eql(u8, path, "/")) {
        return fs.getRootId();
    }

    var current_id = fs.getRootId();
    var iter = std.mem.tokenizeScalar(u8, path, '/');

    while (iter.next()) |component| {
        current_id = fs.lookup(current_id, component) catch |err| {
            return err;
        };
    }

    return current_id;
}

fn cmdLs(fs: *Filesystem, path: []const u8, writer: anytype) !void {
    const dir_id = resolvePath(fs, path) catch |err| {
        if (err == error.NotFound) {
            try writer.print("Error: path not found: {s}\n", .{path});
            return;
        }
        return err;
    };

    const attrs = try fs.getAttrs(dir_id);
    if (attrs.item_type != @intFromEnum(ItemType.directory)) {
        try writer.print("Error: not a directory: {s}\n", .{path});
        return;
    }

    const entries = try fs.readDir(dir_id);
    defer fs.freeDirEntries(entries);

    for (entries) |entry| {
        const type_char: u8 = switch (entry.item_type) {
            .file => '-',
            .directory => 'd',
            .symlink => 'l',
        };

        const entry_attrs = try fs.getAttrs(entry.inode_id);
        try writer.print("{c} {o} {d:>8} {s}\n", .{
            type_char,
            entry_attrs.mode,
            entry_attrs.size,
            entry.name,
        });
    }
}

fn cmdCat(fs: *Filesystem, allocator: std.mem.Allocator, path: []const u8, writer: anytype) !void {
    const file_id = resolvePath(fs, path) catch |err| {
        if (err == error.NotFound) {
            try writer.print("Error: file not found: {s}\n", .{path});
            return;
        }
        return err;
    };

    const attrs = try fs.getAttrs(file_id);
    if (attrs.item_type != @intFromEnum(ItemType.file)) {
        try writer.print("Error: not a file: {s}\n", .{path});
        return;
    }

    if (attrs.size == 0) return;

    const buf = try allocator.alloc(u8, attrs.size);
    defer allocator.free(buf);

    const bytes_read = try fs.read(file_id, 0, buf);
    try writer.writeAll(buf[0..bytes_read]);
}

fn printTree(fs: *Filesystem, allocator: std.mem.Allocator, dir_id: u64, prefix: []const u8, writer: anytype) !void {
    const entries = try fs.readDir(dir_id);
    defer fs.freeDirEntries(entries);

    for (entries, 0..) |entry, i| {
        const is_last = i == entries.len - 1;

        try writer.writeAll(prefix);
        try writer.writeAll(if (is_last) "└── " else "├── ");

        const suffix: []const u8 = switch (entry.item_type) {
            .directory => "/",
            .symlink => "@",
            .file => "",
        };
        try writer.print("{s}{s}\n", .{ entry.name, suffix });

        if (entry.item_type == .directory) {
            var new_prefix: std.ArrayList(u8) = .{};
            defer new_prefix.deinit(allocator);
            try new_prefix.appendSlice(allocator, prefix);
            try new_prefix.appendSlice(allocator, if (is_last) "    " else "│   ");
            try printTree(fs, allocator, entry.inode_id, new_prefix.items, writer);
        }
    }
}

fn cmdTree(fs: *Filesystem, allocator: std.mem.Allocator, writer: anytype) !void {
    try writer.writeAll("/\n");
    try printTree(fs, allocator, fs.getRootId(), "", writer);
}

fn cmdInfo(fs: *Filesystem, allocator: std.mem.Allocator, writer: anytype) !void {
    const Stats = struct { files: u64 = 0, dirs: u64 = 0, symlinks: u64 = 0, size: u64 = 0 };

    const collectStats = struct {
        fn collect(f: *Filesystem, alloc: std.mem.Allocator, dir_id: u64, stats: *Stats) !void {
            const entries = try f.readDir(dir_id);
            defer f.freeDirEntries(entries);

            for (entries) |entry| {
                const attrs = try f.getAttrs(entry.inode_id);
                switch (entry.item_type) {
                    .file => {
                        stats.files += 1;
                        stats.size += attrs.size;
                    },
                    .directory => {
                        stats.dirs += 1;
                        try collect(f, alloc, entry.inode_id, stats);
                    },
                    .symlink => stats.symlinks += 1,
                }
            }
        }
    }.collect;

    var stats = Stats{};
    try collectStats(fs, allocator, fs.getRootId(), &stats);

    try writer.print("Files:       {d}\n", .{stats.files});
    try writer.print("Directories: {d}\n", .{stats.dirs});
    try writer.print("Symlinks:    {d}\n", .{stats.symlinks});
    try writer.print("Total size:  {d} bytes\n", .{stats.size});
}

// =============================================================================
// Main
// =============================================================================

// Exit helper that flushes before exit (std.process.exit skips defers)
fn exitWithCode(stdout: anytype, stderr: anytype, code: u8) noreturn {
    stdout.flush() catch {};
    stderr.flush() catch {};
    std.process.exit(code);
}

pub fn main() !void {
    var gpa = std.heap.GeneralPurposeAllocator(.{}){};
    defer _ = gpa.deinit();
    const allocator = gpa.allocator();

    // Zig 0.15 writer API requires buffers - pass .interface for std.Io.Writer methods
    var stdout_buf: [4096]u8 = undefined;
    var stderr_buf: [4096]u8 = undefined;
    var stdout_writer = std.fs.File.stdout().writer(&stdout_buf);
    var stderr_writer = std.fs.File.stderr().writer(&stderr_buf);
    const stdout = &stdout_writer.interface;
    const stderr = &stderr_writer.interface;
    defer stdout.flush() catch {};
    defer stderr.flush() catch {};

    const args = try std.process.argsAlloc(allocator);
    defer std.process.argsFree(allocator, args);

    if (args.len < 2) {
        try printUsage(stdout);
        exitWithCode(stdout, stderr, 0);
    }

    const cmd = parseCommand(args[1]) orelse {
        try stderr.print("Error: unknown command '{s}'\n\n", .{args[1]});
        try printUsage(stderr);
        exitWithCode(stdout, stderr, 1);
    };

    switch (cmd) {
        .help => {
            try printUsage(stdout);
        },

        // Overlay commands
        .init => {
            if (args.len < 3) {
                try stderr.writeAll("Error: missing path argument\n");
                try stderr.writeAll("Usage: loaf init <path> [overlay.loaf]\n");
                exitWithCode(stdout, stderr, 1);
            }
            const overlay_path = if (args.len >= 4) args[3] else null;
            try cmdInit(allocator, args[2], overlay_path, stdout);
        },

        .diff => {
            const overlay_path = if (args.len >= 3)
                try allocator.dupeZ(u8, args[2])
            else
                findDefaultOverlay(allocator) orelse {
                    try stderr.writeAll("Error: no overlay found. Specify path or run from overlay directory.\n");
                    exitWithCode(stdout, stderr, 1);
                };
            defer allocator.free(overlay_path);
            try cmdDiff(allocator, overlay_path, stdout);
        },

        .accept => {
            const overlay_path = if (args.len >= 3)
                try allocator.dupeZ(u8, args[2])
            else
                findDefaultOverlay(allocator) orelse {
                    try stderr.writeAll("Error: no overlay found.\n");
                    exitWithCode(stdout, stderr, 1);
                };
            defer allocator.free(overlay_path);
            try cmdAccept(allocator, overlay_path, stdout);
        },

        .reject => {
            const overlay_path = if (args.len >= 3)
                try allocator.dupeZ(u8, args[2])
            else
                findDefaultOverlay(allocator) orelse {
                    try stderr.writeAll("Error: no overlay found.\n");
                    exitWithCode(stdout, stderr, 1);
                };
            defer allocator.free(overlay_path);
            try cmdReject(allocator, overlay_path, stdout);
        },

        .status => {
            try cmdStatus(allocator, stdout);
        },

        .run => {
            if (args.len < 3) {
                try stderr.writeAll("Error: missing command\n");
                try stderr.writeAll("Usage: loaf run <cmd> [args...]\n");
                exitWithCode(stdout, stderr, 1);
            }
            const exit_code = try cmdRun(allocator, args[2..], stdout, stderr);
            exitWithCode(stdout, stderr, exit_code);
        },

        // FSKit mount commands
        .mount => {
            if (args.len < 4) {
                try stderr.writeAll("Error: missing arguments\n");
                try stderr.writeAll("Usage: loaf mount <overlay.loaf> <mount-point>\n");
                exitWithCode(stdout, stderr, 1);
            }
            const exit_code = try cmdMount(allocator, args[2], args[3], stdout, stderr);
            exitWithCode(stdout, stderr, exit_code);
        },

        .unmount => {
            if (args.len < 3) {
                try stderr.writeAll("Error: missing mount point\n");
                try stderr.writeAll("Usage: loaf unmount <mount-point>\n");
                exitWithCode(stdout, stderr, 1);
            }
            const exit_code = try cmdUnmount(allocator, args[2], stdout, stderr);
            exitWithCode(stdout, stderr, exit_code);
        },

        // Inspection commands
        .ls, .cat, .tree, .info => {
            // Try to find default .loaf, otherwise require explicit path
            const default_db = findDefaultOverlay(allocator);
            const has_default = default_db != null;
            const db_path = default_db orelse if (args.len >= 3)
                try allocator.dupeZ(u8, args[2])
            else {
                try stderr.writeAll("Error: no .loaf database found. Specify path or run from overlay directory.\n");
                exitWithCode(stdout, stderr, 1);
            };
            defer allocator.free(db_path);

            // Argument index shifts based on whether we used default db
            const path_arg_idx: usize = if (has_default) 2 else 3;

            var fs = Filesystem.init(allocator, db_path) catch |err| {
                try stderr.print("Error opening database: {s}\n", .{@errorName(err)});
                exitWithCode(stdout, stderr, 1);
            };
            defer fs.deinit();

            switch (cmd) {
                .ls => try cmdLs(fs, if (args.len > path_arg_idx) args[path_arg_idx] else "/", stdout),
                .cat => {
                    if (args.len <= path_arg_idx) {
                        try stderr.writeAll("Error: missing file path\n");
                        exitWithCode(stdout, stderr, 1);
                    }
                    try cmdCat(fs, allocator, args[path_arg_idx], stdout);
                },
                .tree => try cmdTree(fs, allocator, stdout),
                .info => try cmdInfo(fs, allocator, stdout),
                else => unreachable,
            }
        },
    }
}
