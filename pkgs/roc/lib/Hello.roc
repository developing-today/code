module [str]
import Square exposing [f64]
num : F64
num = 8192.0125
str : Str -> Str
str = |language|
    Str.join_with(
        [
            Str.join_with(["Roc loves", language], " "),
            Str.join_with([Num.to_str(num), "^2=", Num.to_str(f64(num))], ""),
            Str.join_with(["This is a Roc application running on ", language, "."], ""),
            "It imports a Roc module and calls a function from it.",
        ],
        "\n",
    )
