module [str]
import Lib.Square as Square
num : F64
num = 8192.0125
str : Str
str = Str.join_with([
  "Roc <3 Go!",
  Str.join_with([Num.to_str(num), "^2=", Num.to_str(Square.f64(num))], ""),
  "This is a Roc application running on Go.",
  "It imports a Roc module and calls a function from it.",
  ], "\n")
