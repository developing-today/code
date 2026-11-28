platform "rust-minimal-cli"
    requires {} { main! : _ => _ }
    exposes []
    packages {}
    imports []
    provides [main_for_host!]
import Arg
import InternalArg
import Effect
main_for_host! = |raw_args|
    when
        main!(
            raw_args
            |> List.map(InternalArg.to_os_raw)
            |> List.map(Arg.from_os_raw),
        )
    is
        Ok({}) -> 0
        Err(Exit(code, str)) ->
            if Str.is_empty(str) then
                code
            else
                Effect.log!(str)
                code

        Err(other) ->
            Effect.log!("Program exited early with error: ${Inspect.to_str(other)}")
            1
