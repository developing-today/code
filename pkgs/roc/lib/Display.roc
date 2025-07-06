module [str]
#module [str, matrix]
str : List (List Str) -> Str
str = |lines| lines
  |> List.map (\ws -> Str.join_with(ws, " "))
  |> Str.join_with("\n")
#matrix : Int -> Int -> List (List Str)
#matrix rows cols =
#    List.range 0 rows
#        |> List.map (\r ->
#            List.range 0 cols
#                |> List.map (\c -> Num.to_str (r * cols + c))
#        )
