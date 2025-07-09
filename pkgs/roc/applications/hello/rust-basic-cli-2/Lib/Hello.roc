module [str]
import Square exposing [f64]
num = 8192.0125
str = |language|
    """
    Roc loves ${language}
    ${Num.to_str(num)}^2=${Num.to_str(f64(num))}
    This is a Roc application running on ${language}.
    It imports a Roc module and calls a function from it.
    """
