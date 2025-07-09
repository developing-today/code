app [main!] {
    pf: platform "./Platform/main.roc",
    lib: "./Lib/main.roc",
}
import lib.Hello exposing [hello]
import pf.Stdout exposing [line!]
language = "Rust"
main! = |_|
    line!(hello language)?
    line!("Roc ❤️  ${language}")?
    Ok({})
