app [main] {
    pf: platform "../../../platforms/go-basic-cli/main.roc",
}
import Lib.Hello as Hello
main : Str
main = Hello.str
