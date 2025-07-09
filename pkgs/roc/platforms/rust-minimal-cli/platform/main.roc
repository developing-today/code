platform "rust-minimal-cli"
    requires {} { main! : _ => _ }
    exposes []
    packages {}
    imports []
    provides [main_for_host!]
import Arg
import InternalArg
main_for_host! = |raw_args|
    main!(
        raw_args
        |> List.map(InternalArg.to_os_raw)
        |> List.map(Arg.from_os_raw),
    )
