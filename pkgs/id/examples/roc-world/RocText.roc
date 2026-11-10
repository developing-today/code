## Bounded text helpers for portable world programs.
##
## These are thin, predictable wrappers over the string operations the pinned
## Roc nightly builds reliably. They exist so apps stop re-deriving byte-level
## trimming, casing, and prefix handling in every program.
##
## All helpers are total: invalid UTF-8 after a byte-level transform becomes
## `""` rather than a crash, and indexes outside a string read as `0`.
RocText := {}.{
	## Remove leading and trailing ASCII space, tab, CR, and LF.
	trim : Str -> Str
	trim = |text| {
		bytes = Str.to_utf8(text)
		start = leading(bytes, 0)
		end = trailing(bytes, List.len(bytes))
		if end <= start {
			""
		} else {
			text_of(List.sublist(bytes, { start: start, len: end - start }))
		}
	}

	## Map ASCII `A`..`Z` to `a`..`z`; every other byte is unchanged.
	lowercase : Str -> Str
	lowercase = |text|
		text_of(List.map(Str.to_utf8(text), |byte| if byte >= 65 and byte <= 90 { byte + 32 } else { byte }))

	## Whether `text` begins with `prefix` (byte comparison).
	starts_with : Str, Str -> Bool
	starts_with = |text, prefix| {
		haystack = Str.to_utf8(text)
		needle = Str.to_utf8(prefix)
		if List.len(needle) > List.len(haystack) {
			Bool.False
		} else {
			List.fold_with_index(
				needle,
				Bool.True,
				|ok, byte, index| ok and byte_at(haystack, index) == byte,
			)
		}
	}

	## `text` without `prefix` when it starts with it, otherwise `text`.
	drop_prefix : Str, Str -> Str
	drop_prefix = |text, prefix|
		if starts_with(text, prefix) {
			bytes = Str.to_utf8(text)
			start = List.len(Str.to_utf8(prefix))
			text_of(List.sublist(bytes, { start: start, len: List.len(bytes) - start }))
		} else {
			text
		}

	## Whether `text` contains `needle`.
	contains : Str, Str -> Bool
	contains = |text, needle| Str.contains(text, needle)

	## Split on newlines. A trailing newline yields a final empty line, which is
	## what `join_lines` round-trips.
	lines : Str -> List(Str)
	lines = |text| Str.split_on(text, "\n")

	## Join lines with newlines.
	join_lines : List(Str) -> Str
	join_lines = |items| Str.join_with(items, "\n")

	## The UTF-8 text of `bytes`, or `""` when they are not valid UTF-8.
	text_of : List(U8) -> Str
	text_of = |bytes| match Str.from_utf8(bytes) {
		Ok(text) => text
		Err(_) => ""
	}

	leading : List(U8), U64 -> U64
	leading = |bytes, index|
		if index < List.len(bytes) and is_space(byte_at(bytes, index)) {
			leading(bytes, index + 1)
		} else {
			index
		}

	trailing : List(U8), U64 -> U64
	trailing = |bytes, index|
		if index > 0 and is_space(byte_at(bytes, index - 1)) {
			trailing(bytes, index - 1)
		} else {
			index
		}

	is_space : U8 -> Bool
	is_space = |byte| byte == 32 or byte == 9 or byte == 13 or byte == 10

	byte_at : List(U8), U64 -> U8
	byte_at = |bytes, index| match List.get(bytes, index) {
		Ok(byte) => byte
		Err(_) => 0
	}
}
