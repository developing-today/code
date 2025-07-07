app [main!] {
    pf: platform "../../../platforms/rust-basic-cli/platform/main.roc",
}
## import Lib.Hello as Hello
## main : Str
## main = Hello.str

import pf.Stdout
main! : {} => Result {} _
main! = |{}|
    Stdout.line!("Roc loves Rust")?
    Ok({})
