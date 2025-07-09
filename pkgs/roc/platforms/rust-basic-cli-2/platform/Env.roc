module [
    var!,
    platform!,
]
import Host
var! : Str => Result Str [VarNotFound]
var! = |name|
    Host.env_var!(name)
    |> Result.map_err(|{}| VarNotFound)
ARCH : [X86, X64, ARM, AARCH64, OTHER Str]
OS : [LINUX, MACOS, WINDOWS, OTHER Str]
platform! : {} => { arch : ARCH, os : OS }
platform! = |{}|
    from_rust = Host.current_arch_os!({})
    arch =
        when from_rust.arch is
            "x86" -> X86
            "x86_64" -> X64
            "arm" -> ARM
            "aarch64" -> AARCH64
            _ -> OTHER(from_rust.arch)
    os =
        when from_rust.os is
            "linux" -> LINUX
            "macos" -> MACOS
            "windows" -> WINDOWS
            _ -> OTHER(from_rust.os)
    { arch, os }
