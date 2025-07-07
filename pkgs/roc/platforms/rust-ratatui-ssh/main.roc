app [main] {
  pf: platform "../../../platforms/rust-ratatui-cli/main.roc"
}
import Lib.Hello as Hello
main : Str
main = Hello.str
