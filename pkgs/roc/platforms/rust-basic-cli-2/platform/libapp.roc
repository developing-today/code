app [main!] { pf: platform "main.roc" }
main! : _ => Result {} [Exit I32 Str]_
main! = |_args|
    Err(JustAStub)
