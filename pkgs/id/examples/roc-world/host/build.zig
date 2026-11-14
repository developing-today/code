const std = @import("std");

pub fn build(b: *std.Build) void {
    const optimize = b.standardOptimizeOption(.{ .preferred_optimize_mode = .ReleaseSmall });
    const target = b.resolveTargetQuery(.{ .cpu_arch = .wasm32, .os_tag = .freestanding, .abi = .none });
    const obj = b.addObject(.{
        .name = "host",
        .root_module = b.createModule(.{
            .root_source_file = b.path("src/host.zig"),
            .target = target,
            .optimize = optimize,
            .strip = optimize != .Debug,
            .pic = true,
        }),
    });
    obj.link_function_sections = true;
    obj.link_data_sections = true;
    obj.bundle_compiler_rt = true;
    const copy = b.addUpdateSourceFiles();
    copy.addCopyFileToSource(obj.getEmittedBin(), "../targets/wasm32/host.wasm");
    b.getInstallStep().dependOn(&copy.step);
}
