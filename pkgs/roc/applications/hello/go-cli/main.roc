app [main] {
  pf: platform "../../../platforms/go-cli/main.roc"
}
import Lib.Hello as Hello
main : Str
main = Hello.str
