app [main!] { pf: platform "https://github.com/roc-lang/basic-cli/releases/download/0.19.0/Hj-J_zxz7V9YurCSTFcFdu6cQJie4guzsPMUi5kBYUk.tar.br" }
import pf.Stdout
import pf.Stderr
import pf.Arg exposing [Arg]
main! : List Arg => Result {} _
main! = |_args|
    Stdout.line!("Hello, world!")?
    Stdout.write!("No newline after me.")?
    Stderr.line!("Hello, error!")?
    Stderr.write!("Err with no newline after.")?
    ["Foo", "Bar", "Baz"]
    |> List.for_each_try!(|str| Stdout.line!(str))
