const std = @import("std");

pub fn build(b: *std.Build) !void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});

    // SQLite dependency from submodule
    const sqlite_dep = b.dependency("zig-sqlite", .{
        .target = target,
        .optimize = optimize,
    });

    // Main library module
    const lib_mod = b.addModule("loaf", .{
        .root_source_file = b.path("src/main.zig"),
        .target = target,
        .optimize = optimize,
        .link_libc = true,
    });
    lib_mod.addImport("sqlite", sqlite_dep.module("sqlite"));

    // Dynamic library for Swift linking
    const dylib = b.addLibrary(.{
        .name = "loaf",
        .linkage = .dynamic,
        .root_module = lib_mod,
    });
    dylib.installHeader(b.path("include/loaf.h"), "loaf.h");
    b.installArtifact(dylib);

    // Static library variant
    const static_mod = b.addModule("loaf_static_mod", .{
        .root_source_file = b.path("src/main.zig"),
        .target = target,
        .optimize = optimize,
        .link_libc = true,
    });
    static_mod.addImport("sqlite", sqlite_dep.module("sqlite"));

    const static_lib = b.addLibrary(.{
        .name = "loaf_static",
        .linkage = .static,
        .root_module = static_mod,
    });
    b.installArtifact(static_lib);

    // Unit tests
    const test_mod = b.addModule("test_mod", .{
        .root_source_file = b.path("src/main.zig"),
        .target = target,
        .optimize = optimize,
        .link_libc = true,
    });
    test_mod.addImport("sqlite", sqlite_dep.module("sqlite"));

    const tests = b.addTest(.{
        .root_module = test_mod,
    });

    const run_tests = b.addRunArtifact(tests);
    const test_step = b.step("test", "Run unit tests");
    test_step.dependOn(&run_tests.step);

    // CLI executable
    const cli_mod = b.addModule("cli_mod", .{
        .root_source_file = b.path("src/cli.zig"),
        .target = target,
        .optimize = optimize,
        .link_libc = true,
    });
    cli_mod.addImport("sqlite", sqlite_dep.module("sqlite"));

    const cli_exe = b.addExecutable(.{
        .name = "loaf",
        .root_module = cli_mod,
    });
    b.installArtifact(cli_exe);

    // Format check step
    const fmt_step = b.step("fmt", "Format source files");
    const fmt = b.addFmt(.{
        .paths = &.{ "src", "build.zig" },
    });
    fmt_step.dependOn(&fmt.step);
}
