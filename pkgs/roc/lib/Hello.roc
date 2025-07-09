module [hello]
import Square exposing [square]
num = 8192.0125
hello = |language|
    """
    Roc loves ${language}
    ${Num.to_str(num)}^2=${Num.to_str(square(num))}
    This is a Roc application running on ${language}.
    It imports a Roc module and calls a function from it.
    """
