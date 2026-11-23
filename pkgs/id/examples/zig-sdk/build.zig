const std = @import("std");

pub fn build(b: *std.Build) void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});

    const sdk_mod = b.addModule("zig-sdk", .{
        .root_source_file = b.path("src/root.zig"),
        .target = target,
        .optimize = optimize,
    });

    // Static library so guests can link the SDK into a
    // wasm32-freestanding module; `zig build -Dtarget=wasm32-freestanding`
    // proves the SDK compiles for the guest target.
    const lib = b.addLibrary(.{
        .name = "zig-sdk",
        .root_module = sdk_mod,
        .linkage = .static,
    });
    b.installArtifact(lib);

    const test_step = b.step("test", "Run the Zig SDK unit tests");
    for ([_][]const u8{ "src/root.zig", "src/json.zig", "src/grid.zig" }) |path| {
        const unit_tests = b.addTest(.{
            .root_module = b.createModule(.{
                .root_source_file = b.path(path),
                .target = b.graph.host,
            }),
        });
        const run = b.addRunArtifact(unit_tests);
        test_step.dependOn(&run.step);
    }
}
