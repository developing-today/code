app [main!] {
    pf: platform "../../../platforms/rust-basic-cli/platform/main.roc",
    lib: "../../../lib/main.roc"
}
import lib.Hello
import pf.Stdout
main! : {} => Result {} _
main! = |{}|
    Stdout.line!(Hello.str)?
    Stdout.line!("Roc ❤️  Rust")?
    Ok({})
