//! Pure helper logic for web-gen, extracted from the binary crate's
//! `main.rs` in step 1 of the pkgs/web-gen crate split (#8960).
//!
//! This crate is a leaf: it depends only on `rand` and `futures` — no
//! Leptos or wasm-only crates — so the helpers are testable on the
//! host target.

use futures::future::ready;
use rand::Rng;
use std::str::FromStr;

pub fn generate_random_number() -> i32 {
    let mut rng = rand::thread_rng();
    rng.gen_range(0..101)
}

// returns array of tuple [(divisor, number_of_times_divisible)]
pub fn generate_divisible_i_from_n_up_to_m(i: i32, n: i32, m: i32) -> Vec<(i32, i32)> {
    let mut result = Vec::new();
    let mut subject = n;
    while subject < m {
        let times_divisible = subject / i;

        result.push((subject, times_divisible));
        subject += 1;
    }
    result
}

pub fn generate_random_number_and_multiply(input: i32) -> i32 {
    generate_random_number() * input
}

pub fn parse_input(input: &str) -> Result<i32, std::num::ParseIntError> {
    i32::from_str(input)
}

pub fn generate_random_number_and_multiply_result(
    input: Result<i32, std::num::ParseIntError>,
) -> Result<String, std::num::ParseIntError> {
    match input {
        Ok(num) => {
            let result = generate_random_number() * num;
            Ok(result.to_string())
        }
        Err(e) => Err(e),
    }
}

pub fn generate_random_number_and_multiply_str_future(
    input: String,
) -> impl std::future::Future<Output = Result<String, String>> {
    let input_number = parse_input(&input);
    let result = generate_random_number_and_multiply_result(input_number);
    ready(match result {
        Ok(num) => Ok(num),
        Err(e) => Err(e.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;

    // Oracle: i32::from_str semantics — plain decimal integers parse,
    // anything else is the std ParseIntError.
    #[test]
    fn parse_input_parses_plain_integers() {
        assert_eq!(parse_input("42"), Ok(42));
        assert_eq!(parse_input("-7"), Ok(-7));
        assert_eq!(parse_input("0"), Ok(0));
    }

    #[test]
    fn parse_input_rejects_non_integers() {
        assert!(parse_input("abc").is_err());
        assert!(parse_input("").is_err());
        assert!(parse_input("1.5").is_err());
        assert!(parse_input(" 42").is_err());
    }

    // Oracle: the generator draws from the range 0..101.
    #[test]
    fn generate_random_number_stays_within_0_to_100() {
        for _ in 0..1_000 {
            let n = generate_random_number();
            assert!((0..=100).contains(&n), "drew {n}, outside 0..=100");
        }
    }

    // Oracle: the result is input scaled by a draw from 0..=100, so it
    // is always a multiple of input within [0, 100 * input].
    #[test]
    fn generate_random_number_and_multiply_scales_a_bounded_draw() {
        for input in [1, 3, 5, 10] {
            for _ in 0..200 {
                let r = generate_random_number_and_multiply(input);
                assert_eq!(r % input, 0, "{r} is not a multiple of {input}");
                assert!(
                    (0..=100 * input).contains(&r),
                    "{r} outside 0..={} for input {input}",
                    100 * input
                );
            }
        }
        assert_eq!(generate_random_number_and_multiply(0), 0);
    }

    // Oracle: subjects run n..m in order, each paired with subject / i
    // (integer division).
    #[test]
    fn generate_divisible_i_from_n_up_to_m_walks_subjects_in_order() {
        assert_eq!(
            generate_divisible_i_from_n_up_to_m(2, 1, 5),
            vec![(1, 0), (2, 1), (3, 1), (4, 2)]
        );
        assert_eq!(
            generate_divisible_i_from_n_up_to_m(5, 10, 13),
            vec![(10, 2), (11, 2), (12, 2)]
        );
    }

    #[test]
    fn generate_divisible_i_from_n_up_to_m_is_empty_when_n_reaches_m() {
        assert!(generate_divisible_i_from_n_up_to_m(3, 7, 7).is_empty());
        assert!(generate_divisible_i_from_n_up_to_m(3, 8, 7).is_empty());
    }

    // Oracle: Ok(num) becomes the decimal string of a bounded draw
    // times num; Err passes through unchanged.
    #[test]
    fn generate_random_number_and_multiply_result_formats_scaled_draw() {
        let s = generate_random_number_and_multiply_result(Ok(4)).expect("Ok input must stay Ok");
        let v: i32 = s.parse().expect("output must be a decimal integer");
        assert_eq!(v % 4, 0, "{v} is not a multiple of 4");
        assert!((0..=400).contains(&v), "{v} outside 0..=400");
    }

    #[test]
    fn generate_random_number_and_multiply_result_propagates_parse_errors() {
        let expected = parse_input("nope").unwrap_err().to_string();
        let err = generate_random_number_and_multiply_result(parse_input("nope"))
            .expect_err("Err input must stay Err");
        assert_eq!(err.to_string(), expected);
    }

    // Oracle: the future resolves to the same result as the
    // synchronous path, with parse errors stringified.
    #[test]
    fn str_future_resolves_scaled_draw_for_valid_input() {
        let s = block_on(generate_random_number_and_multiply_str_future(
            "6".to_string(),
        ))
        .expect("valid input must resolve Ok");
        let v: i32 = s.parse().expect("output must be a decimal integer");
        assert_eq!(v % 6, 0, "{v} is not a multiple of 6");
        assert!((0..=600).contains(&v), "{v} outside 0..=600");
    }

    #[test]
    fn str_future_resolves_parse_error_as_string_for_invalid_input() {
        let expected = parse_input("nope").unwrap_err().to_string();
        let err = block_on(generate_random_number_and_multiply_str_future(
            "nope".to_string(),
        ))
        .expect_err("invalid input must resolve Err");
        assert_eq!(err, expected);
    }
}
