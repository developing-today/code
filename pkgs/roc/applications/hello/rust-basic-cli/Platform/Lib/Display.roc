module [str, matrix]
str : List (List Str) -> Str
str = |lines|
    lines
    |> List.map (|ws| Str.join_with(ws, "\t"))
    |> Str.join_with("\n")
matrix : List I128 -> List (List Str)
matrix = |list|
    List.range { start: At 0, end: At (Result.with_default(List.get(list, 0), 10) - 1) }
    |> List.map
        (|r|
            List.range { start: At 0, end: At (Result.with_default(List.get(list, 1), 10) - 1) }
            |> List.map
                (|c|
                    Str.join_with
                        [
                            Str.repeat
                                "0"
                                (
                                    Str.count_utf8_bytes(
                                        Num.to_str
                                            (
                                                (
                                                    Result.with_default(List.get(list, 0), 10)
                                                    *
                                                    Result.with_default(List.get(list, 1), 10)
                                                )
                                                - 1
                                            ),
                                    )
                                    -
                                    Str.count_utf8_bytes(
                                        Num.to_str
                                            (
                                                r * Result.with_default(List.get(list, 1), 10) + c
                                            ),
                                    )
                                ),
                            Num.to_str (r * Result.with_default(List.get(list, 1), 10) + c),
                        ]
                        ""
                )
        )
