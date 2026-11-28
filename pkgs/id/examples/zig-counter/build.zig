const std = @import("std");

fn sdkModule(
    b: *std.Build,
    target: std.Build.ResolvedTarget,
    optimize: std.builtin.OptimizeMode,
) *std.Build.Module {
    const dep = b.dependency("zig_sdk", .{
        .target = target,
        .optimize = optimize,
    });
    return dep.module("zig-sdk");
}

pub fn build(b: *std.Build) void {
    const optimize = b.standardOptimizeOption(.{ .preferred_optimize_mode = .ReleaseSmall });
    const wasm_target = b.resolveTargetQuery(.{
        .cpu_arch = .wasm32,
        .os_tag = .freestanding,
        .abi = .none,
    });

    // The guest module: an import-free wasm32-freestanding executable
    // with no entry point; the `plaza_*` exports in src/main.zig are
    // the whole surface, and linear memory is exported for the host.
    const guest_mod = b.createModule(.{
        .root_source_file = b.path("src/main.zig"),
        .target = wasm_target,
        .optimize = optimize,
        .strip = true,
    });
    guest_mod.addImport("zig-sdk", sdkModule(b, wasm_target, optimize));

    const guest = b.addExecutable(.{
        .name = "counter",
        .root_module = guest_mod,
    });
    guest.entry = .disabled;
    guest.export_memory = true;
    // Without an entry point the linker would GC the `plaza_*`
    // exports as unreferenced; rdynamic roots every global symbol
    // (the export fns are the only public globals in the module).
    guest.rdynamic = true;
    b.installArtifact(guest);

    // Host tests for the world program (the rust-counter suite's
    // mirror): the pure logic plus the screen/json documents.
    const host_mod = b.createModule(.{
        .root_source_file = b.path("src/main.zig"),
        .target = b.graph.host,
    });
    host_mod.addImport("zig-sdk", sdkModule(b, b.graph.host, .Debug));

    const test_step = b.step("test", "Run the Zig counter world tests");
    const unit_tests = b.addTest(.{ .root_module = host_mod });
    const run = b.addRunArtifact(unit_tests);
    test_step.dependOn(&run.step);
}
