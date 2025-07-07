app [main] {
  pf: platform "../../../platforms/rust-basic-cli/main.roc"
}
import Lib.Hello as Hello
main : Str
main = Hello.str
