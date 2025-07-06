module [str]
str : List (List Str) -> Str
str = |lines| lines
  |> List.map (\ws -> Str.join_with(ws, " "))
  |> Str.join_with("\n")
