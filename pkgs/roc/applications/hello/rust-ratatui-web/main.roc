app [main] {
  pf: platform "../../../platforms/ratatui-cli/main.roc"
}
import Lib.Hello as Hello
main : Str
main = Hello.str
