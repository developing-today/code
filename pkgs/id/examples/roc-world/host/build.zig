const std = @import("std");

pub fn build(b: *std.Build) void {
    // Zig renamed the Optimize enum variants between 0.16
    // (ReleaseSmall/Debug) and 0.17 (release_small/debug); resolve
    // the names at comptime so this build runs on either toolchain
    // (the two spellings already ping-ponged once each: #8152).
    const Optimize = if (@hasDecl(std.builtin, "OptimizeMode"))
        std.builtin.OptimizeMode
    else
        std.lang.Optimize;
    const release_small: Optimize = std.meta.stringToEnum(Optimize, "ReleaseSmall") orelse
        std.meta.stringToEnum(Optimize, "small").?;
    const optimize = b.standardOptimizeOption(.{ .preferred_optimize_mode = release_small });
    const target = b.resolveTargetQuery(.{ .cpu_arch = .wasm32, .os_tag = .freestanding, .abi = .none });
    const obj = b.addObject(.{
        .name = "host",
        .root_module = b.createModule(.{
            .root_source_file = b.path("src/host.zig"),
            .target = target,
            .optimize = optimize,
            .strip = !std.mem.eql(u8, @tagName(optimize), "Debug") and
                !std.mem.eql(u8, @tagName(optimize), "debug"),
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
