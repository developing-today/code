## A small, dependency-free JSON value type for portable world programs.
## Number stores the exact JSON number lexeme, avoiding implicit float loss.
DecodeError := [InvalidJson, InputTooLarge, NestingTooDeep, OutputTooLarge, InvalidNumber, MissingField, WrongType]

Outcome(a) := [Ok(a), Err(DecodeError)]

Json := [Null, Bool(Bool), Number(Str), String(Str), Array(List(Json)), Object(List({ key : Str, value : Json }))].{
	max_input_bytes : U64
	max_input_bytes = 65536
	max_output_bytes : U64
	max_output_bytes = 65536
	max_nesting_depth : U64
	max_nesting_depth = 64

	decode : Str -> Outcome(Json)
	decode = |text| {
		bytes = Str.to_utf8(text)
		if List.len(bytes) > max_input_bytes {
			Err(DecodeError.InputTooLarge)
		} else {
			match parse_value(bytes, skip_space(bytes, 0), 0) {
				Ok(parsed) =>
					if skip_space(bytes, parsed.next) == List.len(bytes) {
						Ok(parsed.value)
					} else {
						Err(DecodeError.InvalidJson)
					}
				Err(error) => Err(error)
			}
		}
	}

	encode : Json -> Outcome(Str)
	encode = |value| match encode_value(value, 0) {
		Err(error) => Err(error)
		Ok(text) =>
			if List.len(Str.to_utf8(text)) > max_output_bytes {
				Err(DecodeError.OutputTooLarge)
			} else {
				Ok(text)
			}
	}

	get : Json, Str -> Outcome(Json)
	get = |value, key| match value {
		Object(fields) => find_field(fields, key)
		_ => Err(DecodeError.WrongType)
	}

	as_string : Json -> Outcome(Str)
	as_string = |value| match value {
		String(text) => Ok(text)
		_ => Err(DecodeError.WrongType)
	}

	as_number : Json -> Outcome(Str)
	as_number = |value| match value {
		Number(text) => Ok(text)
		_ => Err(DecodeError.WrongType)
	}

	as_bool : Json -> Outcome(Bool)
	as_bool = |value| match value {
		Bool(boolean) => Ok(boolean)
		_ => Err(DecodeError.WrongType)
	}

	as_array : Json -> Outcome(List(Json))
	as_array = |value| match value {
		Array(items) => Ok(items)
		_ => Err(DecodeError.WrongType)
	}

	as_object : Json -> Outcome(List({ key : Str, value : Json }))
	as_object = |value| match value {
		Object(fields) => Ok(fields)
		_ => Err(DecodeError.WrongType)
	}

	parse_value : List(U8), U64, U64 -> Outcome({ value : Json, next : U64 })
	parse_value = |bytes, at, depth| {
		match byte_at(bytes, at) {
			110 => parse_literal(bytes, at, "null", Null)
			116 => parse_literal(bytes, at, "true", Bool(Bool.True))
			102 => parse_literal(bytes, at, "false", Bool(Bool.False))
			34 => match parse_string(bytes, at + 1, []) {
				Ok(parsed) => Ok({ value: String(parsed.text), next: parsed.next })
				Err(error) => Err(error)
			}
			91 =>
				if depth >= max_nesting_depth {
					Err(DecodeError.NestingTooDeep)
				} else {
					parse_array(bytes, at + 1, [], depth + 1)
				}
			123 =>
				if depth >= max_nesting_depth {
					Err(DecodeError.NestingTooDeep)
				} else {
					parse_object(bytes, at + 1, [], depth + 1)
				}
			45 => parse_number(bytes, at)
			c if c >= 48 and c <= 57 => parse_number(bytes, at)
			_ => Err(DecodeError.InvalidJson)
		}
	}

	parse_literal : List(U8), U64, Str, Json -> Outcome({ value : Json, next : U64 })
	parse_literal = |bytes, at, literal, value| {
		wanted = Str.to_utf8(literal)
		if bytes_match(bytes, at, wanted, 0) {
			Ok({ value: value, next: at + List.len(wanted) })
		} else {
			Err(DecodeError.InvalidJson)
		}
	}

	parse_array : List(U8), U64, List(Json), U64 -> Outcome({ value : Json, next : U64 })
	parse_array = |bytes, at, items, depth| {
		pos = skip_space(bytes, at)
		if byte_at(bytes, pos) == 93 {
			Ok({ value: Array(items), next: pos + 1 })
		} else {
			match parse_value(bytes, pos, depth) {
				Err(error) => Err(error)
				Ok(parsed) => {
					more = List.append(items, parsed.value)
					after = skip_space(bytes, parsed.next)
					if byte_at(bytes, after) == 44 {
						parse_array(bytes, after + 1, more, depth)
					} else if byte_at(bytes, after) == 93 {
						Ok({ value: Array(more), next: after + 1 })
					} else {
						Err(DecodeError.InvalidJson)
					}
				}
			}
		}
	}

	parse_object : List(U8), U64, List({ key : Str, value : Json }), U64 -> Outcome({ value : Json, next : U64 })
	parse_object = |bytes, at, fields, depth| {
		pos = skip_space(bytes, at)
		if byte_at(bytes, pos) == 125 {
			Ok({ value: Object(fields), next: pos + 1 })
		} else if byte_at(bytes, pos) != 34 {
			Err(DecodeError.InvalidJson)
		} else {
			match parse_string(bytes, pos + 1, []) {
				Err(error) => Err(error)
				Ok(key) => {
					colon = skip_space(bytes, key.next)
					if byte_at(bytes, colon) != 58 {
						Err(DecodeError.InvalidJson)
					} else {
						value_at = skip_space(bytes, colon + 1)
						match parse_value(bytes, value_at, depth) {
							Err(error) => Err(error)
							Ok(parsed) => {
								more = List.append(fields, { key: key.text, value: parsed.value })
								after = skip_space(bytes, parsed.next)
								if byte_at(bytes, after) == 44 {
									parse_object(bytes, after + 1, more, depth)
								} else if byte_at(bytes, after) == 125 {
									Ok({ value: Object(more), next: after + 1 })
								} else {
									Err(DecodeError.InvalidJson)
								}
							}
						}
					}
				}
			}
		}
	}

	parse_string : List(U8), U64, List(U8) -> Outcome({ text : Str, next : U64 })
	parse_string = |bytes, at, out| {
		c = byte_at(bytes, at)
		if c == 34 {
			match Str.from_utf8(out) {
				Ok(text) => Ok({ text: text, next: at + 1 })
				Err(_) => Err(DecodeError.InvalidJson)
			}
		} else if c == 92 {
			escaped = byte_at(bytes, at + 1)
			if escaped == 34 or escaped == 92 or escaped == 47 {
				parse_string(bytes, at + 2, List.append(out, escaped))
			} else if escaped == 98 {
				parse_string(bytes, at + 2, List.append(out, 8))
			} else if escaped == 102 {
				parse_string(bytes, at + 2, List.append(out, 12))
			} else if escaped == 110 {
				parse_string(bytes, at + 2, List.append(out, 10))
			} else if escaped == 114 {
				parse_string(bytes, at + 2, List.append(out, 13))
			} else if escaped == 116 {
				parse_string(bytes, at + 2, List.append(out, 9))
			} else if escaped == 117 {
				match parse_unicode(bytes, at + 2) {
					Ok(decoded) => parse_string(bytes, decoded.next, append_codepoint(out, decoded.codepoint))
					Err(error) => Err(error)
				}
			} else {
				Err(DecodeError.InvalidJson)
			}
		} else if c < 32 or at >= List.len(bytes) {
			Err(DecodeError.InvalidJson)
		} else {
			parse_string(bytes, at + 1, List.append(out, c))
		}
	}

	parse_unicode : List(U8), U64 -> Outcome({ codepoint : U64, next : U64 })
	parse_unicode = |bytes, at| match read_hex4(bytes, at, 0, 0) {
		Err(error) => Err(error)
		Ok(first) =>
			if first.value >= 55296 and first.value <= 56319 {
				if byte_at(bytes, first.next) == 92 and byte_at(bytes, first.next + 1) == 117 {
					match read_hex4(bytes, first.next + 2, 0, 0) {
						Ok(second) if second.value >= 56320 and second.value <= 57343 =>
							Ok({ codepoint: 65536 + (first.value - 55296) * 1024 + (second.value - 56320), next: second.next })
						_ => Err(DecodeError.InvalidJson)
					}
				} else {
					Err(DecodeError.InvalidJson)
				}
			} else if first.value >= 56320 and first.value <= 57343 {
				Err(DecodeError.InvalidJson)
			} else {
				Ok({ codepoint: first.value, next: first.next })
			}
	}

	read_hex4 : List(U8), U64, U64, U64 -> Outcome({ value : U64, next : U64 })
	read_hex4 = |bytes, at, count, value| {
		if count == 4 {
			Ok({ value: value, next: at })
		} else {
			digit = hex_value(byte_at(bytes, at))
			if digit == 255 {
				Err(DecodeError.InvalidJson)
			} else {
				read_hex4(bytes, at + 1, count + 1, value * 16 + digit)
			}
		}
	}

	hex_value : U8 -> U64
	hex_value = |c| if c >= 48 and c <= 57 {
		(c - 48).to_u64()
	} else if c >= 65 and c <= 70 {
		(c - 55).to_u64()
	} else if c >= 97 and c <= 102 {
		(c - 87).to_u64()
	} else {
		255
	}

	append_codepoint : List(U8), U64 -> List(U8)
	append_codepoint = |out, cp| {
		if cp <= 127 {
			List.append(out, u64_to_u8(cp))
		} else if cp <= 2047 {
			List.append(List.append(out, u64_to_u8(192 + cp / 64)), u64_to_u8(128 + cp % 64))
		} else if cp <= 65535 {
			List.append(List.append(List.append(out, u64_to_u8(224 + cp / 4096)), u64_to_u8(128 + (cp / 64) % 64)), u64_to_u8(128 + cp % 64))
		} else {
			List.append(List.append(List.append(List.append(out, u64_to_u8(240 + cp / 262144)), u64_to_u8(128 + (cp / 4096) % 64)), u64_to_u8(128 + (cp / 64) % 64)), u64_to_u8(128 + cp % 64))
		}
	}

	u64_to_u8 : U64 -> U8
	u64_to_u8 = |number| u64_to_u8_help(number, 0)

	u64_to_u8_help : U64, U8 -> U8
	u64_to_u8_help = |remaining, value| {
		if remaining == 0 {
			value
		} else {
			u64_to_u8_help(remaining - 1, value + 1)
		}
	}

	parse_number : List(U8), U64 -> Outcome({ value : Json, next : U64 })
	parse_number = |bytes, at| {
		end = number_end(bytes, at)
		lexeme = bytes_to_str(List.sublist(bytes, { start: at, len: end - at }))
		if end == at or !valid_number(Str.to_utf8(lexeme)) {
			Err(DecodeError.InvalidJson)
		} else {
			Ok({ value: Number(lexeme), next: end })
		}
	}

	number_end : List(U8), U64 -> U64
	number_end = |bytes, at| {
		c = byte_at(bytes, at)
		if (c >= 48 and c <= 57) or c == 45 or c == 43 or c == 46 or c == 69 or c == 101 {
			number_end(bytes, at + 1)
		} else {
			at
		}
	}

	valid_number : List(U8) -> Bool
	valid_number = |bytes| {
		len = List.len(bytes)
		start = if byte_at(bytes, 0) == 45 {
			1
		} else {
			0
		}
		integer_end = if byte_at(bytes, start) == 48 {
			start + 1
		} else if byte_at(bytes, start) >= 49 and byte_at(bytes, start) <= 57 {
			digits_end(bytes, start)
		} else {
			start
		}
		has_integer = integer_end > start
		fraction_end = if byte_at(bytes, integer_end) == 46 and is_digit(byte_at(bytes, integer_end + 1)) {
			digits_end(bytes, integer_end + 1)
		} else {
			integer_end
		}
		has_fraction = fraction_end > integer_end
		exponent_at = fraction_end
		exponent_start = if byte_at(bytes, exponent_at) == 69 or byte_at(bytes, exponent_at) == 101 {
			if byte_at(bytes, exponent_at + 1) == 43 or byte_at(bytes, exponent_at + 1) == 45 {
				exponent_at + 2
			} else {
				exponent_at + 1
			}
		} else {
			exponent_at
		}
		exponent_end = if exponent_start > exponent_at and is_digit(byte_at(bytes, exponent_start)) {
			digits_end(bytes, exponent_start)
		} else {
			exponent_at
		}
		has_exponent = exponent_end > exponent_at
		has_integer and (integer_end == start + 1 or byte_at(bytes, start) != 48 or integer_end == start + 1) and (fraction_end == integer_end or has_fraction) and (exponent_end == exponent_at or has_exponent) and exponent_end == len
	}

	digits_end : List(U8), U64 -> U64
	digits_end = |bytes, at| if is_digit(byte_at(bytes, at)) {
		digits_end(bytes, at + 1)
	} else {
		at
	}

	is_digit : U8 -> Bool
	is_digit = |c| c >= 48 and c <= 57

	find_field : List({ key : Str, value : Json }), Str -> Outcome(Json)
	find_field = |fields, key| match List.first(List.keep_if(fields, |field| field.key == key)) {
		Ok(field) => Ok(field.value)
		Err(_) => Err(DecodeError.MissingField)
	}

	encode_value : Json, U64 -> Outcome(Str)
	encode_value = |value, depth| match value {
		Null => Ok("null")
		Bool(boolean) => Ok(
			if boolean {
				"true"
			} else {
				"false"
			},
		)
		Number(text) =>
			if valid_number(Str.to_utf8(text)) {
				Ok(text)
			} else {
				Err(DecodeError.InvalidNumber)
			}
		String(text) => Ok(quote(text))
		Array(items) =>
			if depth >= max_nesting_depth {
				Err(DecodeError.NestingTooDeep)
			} else {
				match encode_values(items, depth + 1, []) {
					Ok(encoded) => Ok(Str.concat("[", Str.concat(Str.join_with(encoded, ","), "]")))
					Err(error) => Err(error)
				}
			}
		Object(fields) =>
			if depth >= max_nesting_depth {
				Err(DecodeError.NestingTooDeep)
			} else {
				match encode_fields(fields, depth + 1, []) {
					Ok(encoded) => Ok(Str.concat("{", Str.concat(Str.join_with(encoded, ","), "}")))
					Err(error) => Err(error)
				}
			}
	}

	encode_values : List(Json), U64, List(Str) -> Outcome(List(Str))
	encode_values = |values, depth, encoded| match List.first(values) {
		Err(_) => Ok(encoded)
		Ok(value) => match encode_value(value, depth) {
			Err(error) => Err(error)
			Ok(text) => encode_values(List.drop_first(values, 1), depth, List.append(encoded, text))
		}
	}

	encode_fields : List({ key : Str, value : Json }), U64, List(Str) -> Outcome(List(Str))
	encode_fields = |fields, depth, encoded| match List.first(fields) {
		Err(_) => Ok(encoded)
		Ok(field) => match encode_value(field.value, depth) {
			Err(error) => Err(error)
			Ok(text) => {
				pair = Str.concat(quote(field.key), Str.concat(":", text))
				encode_fields(List.drop_first(fields, 1), depth, List.append(encoded, pair))
			}
		}
	}

	quote : Str -> Str
	quote = |text| {
		escaped = List.fold(
			Str.to_utf8(text),
			[],
			|out, byte| {
				chunk = if byte == 34 {
					[92, 34]
				} else if byte == 92 {
					[92, 92]
				} else if byte == 8 {
					[92, 98]
				} else if byte == 9 {
					[92, 116]
				} else if byte == 10 {
					[92, 110]
				} else if byte == 12 {
					[92, 102]
				} else if byte == 13 {
					[92, 114]
				} else if byte < 32 {
					[92, 117, 48, 48, hex_digit(byte / 16), hex_digit(byte % 16)]
				} else {
					[byte]
				}
				List.fold(chunk, out, |acc, c| List.append(acc, c))
			},
		)
		Str.concat("\"", Str.concat(bytes_to_str(escaped), "\""))
	}

	hex_digit : U8 -> U8
	hex_digit = |value| if value < 10 {
		48 + value
	} else {
		87 + value
	}

	bytes_to_str : List(U8) -> Str
	bytes_to_str = |bytes| match Str.from_utf8(bytes) {
		Ok(text) => text
		Err(_) => ""
	}

	byte_at : List(U8), U64 -> U8
	byte_at = |bytes, at| List.get(bytes, at) ?? 0

	skip_space : List(U8), U64 -> U64
	skip_space = |bytes, at| {
		c = byte_at(bytes, at)
		if c == 32 or c == 9 or c == 10 or c == 13 {
			skip_space(bytes, at + 1)
		} else {
			at
		}
	}

	bytes_match : List(U8), U64, List(U8), U64 -> Bool
	bytes_match = |bytes, at, wanted, i| {
		if i == List.len(wanted) {
			Bool.True
		} else if byte_at(bytes, at + i) == byte_at(wanted, i) {
			bytes_match(bytes, at, wanted, i + 1)
		} else {
			Bool.False
		}
	}

	decoded_as : Str, Json -> Bool
	decoded_as = |text, expected| match decode(text) {
		Ok(value) => same_json(value, expected)
		Err(_) => Bool.False
	}

	same_json : Json, Json -> Bool
	same_json = |left, right| match encode(left) {
		Err(_) => Bool.False
		Ok(left_text) => match encode(right) {
			Err(_) => Bool.False
			Ok(right_text) => left_text == right_text
		}
	}

	invalid_encoding : Json, DecodeError -> Bool
	invalid_encoding = |value, wanted| match encode(value) {
		Err(error) => error_code(error) == error_code(wanted)
		Ok(_) => Bool.False
	}

	depth_text : U64 -> Str
	depth_text = |depth| Str.concat(Str.join_with(List.repeat("[", depth), ""), Str.concat("0", Str.join_with(List.repeat("]", depth), "")))

	object_depth_text : U64 -> Str
	object_depth_text = |depth| Str.concat(Str.join_with(List.repeat("{\"a\":", depth), ""), Str.concat("0", Str.join_with(List.repeat("}", depth), "")))

	input_with_padding : U64 -> Str
	input_with_padding = |padding| {
		bytes = List.concat(Str.to_utf8("null"), List.repeat(32, padding))
		bytes_to_str(bytes)
	}

	decode_error_is : Str, DecodeError -> Bool
	decode_error_is = |text, wanted| match decode(text) {
		Err(error) => error_code(error) == error_code(wanted)
		Ok(_) => Bool.False
	}

	error_code : DecodeError -> U8
	error_code = |error| match error {
		DecodeError.InvalidJson => 0
		DecodeError.InputTooLarge => 1
		DecodeError.NestingTooDeep => 2
		DecodeError.OutputTooLarge => 3
		DecodeError.InvalidNumber => 4
		DecodeError.MissingField => 5
		DecodeError.WrongType => 6
	}

	nested_value : U64, Json -> Json
	nested_value = |depth, value| if depth == 0 {
		value
	} else {
		nested_value(depth - 1, Json.Array([value]))
	}

	nested_object_value : U64, Json -> Json
	nested_object_value = |depth, value| if depth == 0 {
		value
	} else {
		nested_object_value(depth - 1, Json.Object([{ key: "a", value: value }]))
	}

	encoded_as : Json, Str -> Bool
	encoded_as = |value, expected| match encode(value) {
		Ok(text) => text == expected
		Err(_) => Bool.False
	}

	round_trip_matches : Json -> Bool
	round_trip_matches = |value| match encode(value) {
		Ok(text) => decoded_as(text, value)
		Err(_) => Bool.False
	}

	exceeds_depth : Json -> Bool
	exceeds_depth = |value| match encode(value) {
		Err(DecodeError.NestingTooDeep) => Bool.True
		_ => Bool.False
	}

	too_large : Str -> Bool
	too_large = |text| match decode(text) {
		Err(DecodeError.InputTooLarge) => Bool.True
		_ => Bool.False
	}

	too_deep : Str -> Bool
	too_deep = |text| match decode(text) {
		Err(DecodeError.NestingTooDeep) => Bool.True
		_ => Bool.False
	}

	output_too_large : Json -> Bool
	output_too_large = |value| match encode(value) {
		Err(DecodeError.OutputTooLarge) => Bool.True
		_ => Bool.False
	}

	invalid : Str -> Bool
	invalid = |text| match decode(text) {
		Ok(_) => Bool.False
		Err(DecodeError.InvalidJson) => Bool.True
		Err(_) => Bool.False
	}

	accessor_checks : Bool
	accessor_checks = {
		nested = decode("{\"items\":[\"ok\"]}")
		match nested {
			Err(_) => Bool.False
			Ok(root) => match get(root, "items") {
				Err(_) => Bool.False
				Ok(items) => match as_array(items) {
					Err(_) => Bool.False
					Ok(values) => match List.first(values) {
						Err(_) => Bool.False
						Ok(first) => match as_string(first) {
							Ok(text) => text == "ok" and wrong_type(as_bool(first)) and missing_field(get(root, "absent"))
							Err(_) => Bool.False
						}
					}
				}
			}
		}
	}

	wrong_type : Outcome(Bool) -> Bool
	wrong_type = |result| match result {
		Err(DecodeError.WrongType) => Bool.True
		_ => Bool.False
	}

	missing_field : Outcome(Json) -> Bool
	missing_field = |result| match result {
		Err(DecodeError.MissingField) => Bool.True
		_ => Bool.False
	}
}

expect Json.encoded_as(Json.String("a\"\\\n\u(1)"), "\"a\\\"\\\\\\n\\u0001\"")
expect Json.decoded_as("{\"outer\":[null,true,{\"n\":-12.5e+2,\"s\":\"quote: \\\" slash: \\\\ tab:\\t line:\\n\"}]}", Json.Object([{ key: "outer", value: Json.Array([Json.Null, Json.Bool(Bool.True), Json.Object([{ key: "n", value: Json.Number("-12.5e+2") }, { key: "s", value: Json.String("quote: \" slash: \\ tab:\t line:\n") }])]) }]))
expect Json.decoded_as("\"\\uD83D\\uDE03\"", Json.String("😃"))
expect Json.invalid("[1,")
expect Json.invalid("{\"x\":\"unfinished}")
expect Json.invalid("true false")
expect Json.invalid("01")
expect Json.invalid("1e+")
expect Json.accessor_checks
round_trip_sample = Json.Array([Json.Null, Json.Bool(Bool.False), Json.Number("0.125"), Json.String("x\u(8)\t\\\""), Json.Object([{ key: "k", value: Json.Array([]) }])])
expect Json.round_trip_matches(round_trip_sample)
expect Json.invalid_encoding(Json.Number("01"), DecodeError.InvalidNumber)
expect Json.invalid_encoding(Json.Object([{ key: "n", value: Json.Number("1e+") }]), DecodeError.InvalidNumber)
expect Json.encoded_as(Json.Number("-0.25E+3"), "-0.25E+3")
expect Json.decoded_as(Json.depth_text(64), Json.nested_value(64, Json.Number("0")))
expect Json.too_deep(Json.depth_text(65))
expect Json.decoded_as(Json.object_depth_text(64), Json.nested_object_value(64, Json.Number("0")))
expect Json.too_deep(Json.object_depth_text(65))
expect Json.decoded_as(Json.input_with_padding(65532), Json.Null)
expect Json.too_large(Json.input_with_padding(65533))
expect Json.round_trip_matches(Json.nested_value(64, Json.Null))
expect Json.exceeds_depth(Json.nested_value(65, Json.Null))
expect Json.round_trip_matches(Json.nested_object_value(64, Json.Null))
expect Json.exceeds_depth(Json.nested_object_value(65, Json.Null))
expect Json.output_too_large(Json.String(Json.bytes_to_str(List.repeat(65, 65535))))
