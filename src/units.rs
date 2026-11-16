use crate::error::{PackError, PackResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cmp::Ordering;

#[derive(
    Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Length(pub i64);

impl Length {
    pub const TICKS_PER_MM: i64 = 16_000;
    pub const TICKS_PER_INCH: i64 = 406_400;

    pub fn parse(value: &Value, default_unit: &str) -> PackResult<Self> {
        bounded_ticks(value, default_unit, MeasureKind::Length).map(Self)
    }

    pub fn to_json(self, unit: &str) -> Value {
        let multiplier = match unit.to_ascii_lowercase().as_str() {
            "ticks" | "tick" => 1,
            "in" | "inch" | "inches" => Self::TICKS_PER_INCH,
            "cm" => Self::TICKS_PER_MM * 10,
            "m" => Self::TICKS_PER_MM * 1_000,
            "ft" => Self::TICKS_PER_INCH * 12,
            _ => Self::TICKS_PER_MM,
        };
        serde_json::json!({
            "ticks": self.0,
            "value": decimal_string(self.0 as i128, multiplier as i128, 8),
            "unit": unit,
        })
    }
}

#[derive(
    Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Weight(pub i64);

impl Weight {
    pub const TICKS_PER_MG: i64 = 8_000;
    pub const TICKS_PER_G: i64 = 8_000_000;
    pub const TICKS_PER_KG: i64 = 8_000_000_000;
    pub const TICKS_PER_OZ: i64 = 226_796_185;
    pub const TICKS_PER_LB: i64 = 3_628_738_960;

    pub fn parse(value: &Value, default_unit: &str) -> PackResult<Self> {
        bounded_ticks(value, default_unit, MeasureKind::Weight).map(Self)
    }

    pub fn to_json(self, unit: &str) -> Value {
        let multiplier = match unit.to_ascii_lowercase().as_str() {
            "ticks" | "tick" => 1,
            "mg" => Self::TICKS_PER_MG,
            "kg" => Self::TICKS_PER_KG,
            "oz" => Self::TICKS_PER_OZ,
            "lb" | "lbs" => Self::TICKS_PER_LB,
            _ => Self::TICKS_PER_G,
        };
        serde_json::json!({
            "ticks": self.0,
            "value": decimal_string(self.0 as i128, multiplier as i128, 8),
            "unit": unit,
        })
    }
}

/// Which quantity a measure is, so the one parser can tell a length unit from a weight unit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MeasureKind {
    Length,
    Weight,
}

impl MeasureKind {
    /// Ticks per `unit`, matched ignoring case and surrounding space as the reference does.
    fn multiplier(self, unit: &str) -> Option<i64> {
        let unit = unit.trim().to_ascii_lowercase();
        Some(match (self, unit.as_str()) {
            (_, "tick" | "ticks") => 1,
            (Self::Length, "mm" | "millimeter" | "millimeters") => Length::TICKS_PER_MM,
            (Self::Length, "cm") => Length::TICKS_PER_MM * 10,
            (Self::Length, "m") => Length::TICKS_PER_MM * 1_000,
            (Self::Length, "in" | "inch" | "inches") => Length::TICKS_PER_INCH,
            (Self::Length, "ft") => Length::TICKS_PER_INCH * 12,
            (Self::Weight, "mg") => Weight::TICKS_PER_MG,
            (Self::Weight, "g") => Weight::TICKS_PER_G,
            (Self::Weight, "kg") => Weight::TICKS_PER_KG,
            (Self::Weight, "oz") => Weight::TICKS_PER_OZ,
            (Self::Weight, "lb" | "lbs") => Weight::TICKS_PER_LB,
            _ => return None,
        })
    }

    /// The unit words a measure written as text may end with, longest first so `lbs` is not
    /// read as `s` after `lb`. The reference lists exactly these -- not `millimeter(s)`, which
    /// it knows only as a stated unit -- and matches them ignoring case.
    fn text_suffixes(self) -> &'static [&'static str] {
        match self {
            Self::Length => &[
                "inches", "ticks", "inch", "tick", "mm", "cm", "ft", "in", "m",
            ],
            Self::Weight => &["ticks", "tick", "lbs", "mg", "kg", "oz", "lb", "g"],
        }
    }

    pub(crate) fn knows_unit(self, unit: &str) -> bool {
        self.multiplier(unit).is_some()
    }
}

/// The output unit a request without `output.length_unit` is written in: the
/// request's own `units.length`, spelt as the schema's output code, so `Inches` is `in`. A
/// unit no output code names -- the internal `tick` -- gives `mm`, the default of both blocks.
pub(crate) fn output_length_unit(request_unit: &str) -> &'static str {
    let multiplier = MeasureKind::Length.multiplier(request_unit);
    ["mm", "cm", "m", "in", "ft"]
        .into_iter()
        .find(|code| MeasureKind::Length.multiplier(code) == multiplier)
        .unwrap_or("mm")
}

/// A measure's ticks before any range check, so a caller can tell a negative measure from
/// one that does not parse at all. The text of the scalar comes back for error messages.
fn unbounded_ticks(
    value: &Value,
    default_unit: &str,
    kind: MeasureKind,
) -> PackResult<(String, i128)> {
    let (raw, unit) = scalar_and_unit(value, default_unit, kind)?;
    let multiplier = kind
        .multiplier(&unit)
        .ok_or_else(|| PackError::UnsupportedUnit(unit.trim().to_ascii_lowercase()))?;
    let (numerator, denominator) = parse_rational(&raw)?;
    let scaled = numerator
        .checked_mul(multiplier as i128)
        .ok_or_else(|| PackError::InvalidNumber(raw.clone()))?;
    let ticks = round_half_even(scaled, denominator)?;
    Ok((raw, ticks))
}

fn bounded_ticks(value: &Value, default_unit: &str, kind: MeasureKind) -> PackResult<i64> {
    let (raw, ticks) = unbounded_ticks(value, default_unit, kind)?;
    i64::try_from(ticks)
        .ok()
        .filter(|ticks| *ticks >= 0)
        .ok_or(PackError::InvalidNumber(raw))
}

/// The measure's ticks, negative ones included; an error only when it does not parse.
pub(crate) fn measure_ticks(
    value: &Value,
    default_unit: &str,
    kind: MeasureKind,
) -> PackResult<i128> {
    unbounded_ticks(value, default_unit, kind).map(|(_, ticks)| ticks)
}

/// What a measure `measure_ticks` overflowed on is, read exactly.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Overflowed {
    Negative,
    /// Its ticks are past the ceiling the caller named.
    Beyond,
    /// Unreadable here too, or a vanishing fraction with a vast denominator.
    Unreadable,
}

/// Reads over decimal digits a measure `i128` could not hold, to say whether it is negative or
/// past `ceiling` ticks. Ticks round to nearest, so they pass an odd `ceiling` once
/// `2 n m >= (2 ceiling + 1) d`; the tie itself rounds up to even. O(digits^2).
pub(crate) fn overflowed_measure(
    value: &Value,
    default_unit: &str,
    kind: MeasureKind,
    ceiling: u64,
) -> Overflowed {
    let Ok((raw, unit)) = scalar_and_unit(value, default_unit, kind) else {
        return Overflowed::Unreadable;
    };
    let (Some(multiplier), Some((negative, numerator, denominator))) =
        (kind.multiplier(&unit), exact_fraction(&raw))
    else {
        return Overflowed::Unreadable;
    };
    if numerator.is_empty() {
        return Overflowed::Unreadable;
    }
    if negative {
        return Overflowed::Negative;
    }
    let doubled = digits_mul(&numerator, &digits_of(2 * multiplier as u128));
    let threshold = digits_mul(&denominator, &digits_of(2 * u128::from(ceiling) + 1));
    if digits_cmp(&doubled, &threshold) == Ordering::Less {
        Overflowed::Unreadable
    } else {
        Overflowed::Beyond
    }
}

/// How a measure's number is written. Python's grammar, and so every engine's: an optional
/// sign, ASCII digits, and either a `.` with digits on both sides (at most
/// `MAX_FRACTIONAL_DIGITS` after it), a `/` over digits, or a whole part then a fraction after
/// whitespace. Nothing else -- no `1e2`, `0x64`, `1_0`, `100.`, `.5` -- and no sign inside a
/// fraction. All pieces are digit runs.
#[derive(Debug, Eq, PartialEq)]
enum Written<'a> {
    /// `whole` alone or `whole.fraction`; the fraction is empty for a whole number.
    Decimal {
        negative: bool,
        whole: &'a str,
        fraction: &'a str,
    },
    /// `numerator / denominator`.
    Ratio {
        negative: bool,
        numerator: &'a str,
        denominator: &'a str,
    },
    /// `whole numerator/denominator`; the sign of the text signs the whole value.
    Mixed {
        negative: bool,
        whole: &'a str,
        numerator: &'a str,
        denominator: &'a str,
    },
}

/// Past this many fractional digits a decimal is refused, the cap the reference shares with PHP.
const MAX_FRACTIONAL_DIGITS: usize = 18;

/// The whitespace of the reference's `\s` over ASCII text (space, tab, newline, return, form feed, vertical tab).
fn is_written_space(character: char) -> bool {
    matches!(character, ' ' | '\t' | '\n' | '\r' | '\x0b' | '\x0c')
}

/// The leading run of ASCII digits, which must not be empty, and what follows it.
fn digit_run(text: &str) -> Option<(&str, &str)> {
    let length = text.bytes().take_while(u8::is_ascii_digit).count();
    (length > 0).then(|| text.split_at(length))
}

/// The digits of `/ digits` ending the text, allowing whitespace around the slash.
fn denominator_digits(text: &str) -> Option<&str> {
    let after = text
        .trim_start_matches(is_written_space)
        .strip_prefix('/')?
        .trim_start_matches(is_written_space);
    digit_run(after)
        .filter(|(_, rest)| rest.is_empty())
        .map(|(digits, _)| digits)
}

/// `text` (already trimmed, its no-break spaces made plain) read in the measure grammar, or
/// `None` when it is not a number at all. O(len(text)).
fn read_written(text: &str) -> Option<Written<'_>> {
    let (negative, unsigned) = match text.as_bytes().first()? {
        b'-' => (true, &text[1..]),
        b'+' => (false, &text[1..]),
        _ => (false, text),
    };
    let (whole, rest) = digit_run(unsigned)?;
    if rest.is_empty() {
        return Some(Written::Decimal {
            negative,
            whole,
            fraction: "",
        });
    }
    if let Some(after_point) = rest.strip_prefix('.') {
        let (fraction, tail) = digit_run(after_point)?;
        return (tail.is_empty() && fraction.len() <= MAX_FRACTIONAL_DIGITS).then_some(
            Written::Decimal {
                negative,
                whole,
                fraction,
            },
        );
    }
    let after_space = rest.trim_start_matches(is_written_space);
    if after_space.starts_with('/') {
        return Some(Written::Ratio {
            negative,
            numerator: whole,
            denominator: denominator_digits(after_space)?,
        });
    }
    if after_space.len() == rest.len() {
        return None;
    }
    let (numerator, tail) = digit_run(after_space)?;
    Some(Written::Mixed {
        negative,
        whole,
        numerator,
        denominator: denominator_digits(tail)?,
    })
}

/// A measure's text as the grammar reads it: trimmed, no-break spaces plain.
fn normalized(text: &str) -> String {
    text.trim().replace('\u{00a0}', " ")
}

/// `parse_rational` over little-endian decimal digits, without leading zeros: the sign,
/// numerator and denominator, or `None` for any other text.
fn exact_fraction(text: &str) -> Option<(bool, Vec<u8>, Vec<u8>)> {
    let text = normalized(text);
    let (negative, numerator, denominator) = match read_written(&text)? {
        Written::Decimal {
            negative,
            whole,
            fraction,
        } => (
            negative,
            digits(&format!("{whole}{fraction}"))?,
            digits(&format!("1{}", "0".repeat(fraction.len())))?,
        ),
        Written::Ratio {
            negative,
            numerator,
            denominator,
        } => (negative, digits(numerator)?, digits(denominator)?),
        Written::Mixed {
            negative,
            whole,
            numerator,
            denominator,
        } => {
            let denominator = digits(denominator)?;
            let whole = digits_mul(&digits(whole)?, &denominator);
            (
                negative,
                digits_add(&whole, &digits(numerator)?),
                denominator,
            )
        }
    };
    if denominator.is_empty() {
        return None;
    }
    Some((negative && !numerator.is_empty(), numerator, denominator))
}

fn digits(text: &str) -> Option<Vec<u8>> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let mut digits: Vec<u8> = text.bytes().rev().map(|byte| byte - b'0').collect();
    while digits.last() == Some(&0) {
        digits.pop();
    }
    Some(digits)
}

fn digits_of(value: u128) -> Vec<u8> {
    digits(&value.to_string()).unwrap_or_default()
}

fn digits_add(a: &[u8], b: &[u8]) -> Vec<u8> {
    let mut sum = Vec::with_capacity(a.len().max(b.len()) + 1);
    let mut carry = 0;
    for index in 0..a.len().max(b.len()) {
        let digit = a.get(index).unwrap_or(&0) + b.get(index).unwrap_or(&0) + carry;
        sum.push(digit % 10);
        carry = digit / 10;
    }
    if carry > 0 {
        sum.push(carry);
    }
    sum
}

fn digits_mul(a: &[u8], b: &[u8]) -> Vec<u8> {
    let mut product = vec![0_u32; a.len() + b.len()];
    for (i, &left) in a.iter().enumerate() {
        for (j, &right) in b.iter().enumerate() {
            product[i + j] += u32::from(left) * u32::from(right);
        }
    }
    let mut carry = 0;
    for digit in &mut product {
        *digit += carry;
        carry = *digit / 10;
        *digit %= 10;
    }
    let mut product: Vec<u8> = product.into_iter().map(|digit| digit as u8).collect();
    while product.last() == Some(&0) {
        product.pop();
    }
    product
}

fn digits_cmp(a: &[u8], b: &[u8]) -> Ordering {
    a.len()
        .cmp(&b.len())
        .then_with(|| a.iter().rev().cmp(b.iter().rev()))
}

fn scalar_and_unit(
    value: &Value,
    default_unit: &str,
    kind: MeasureKind,
) -> PackResult<(String, String)> {
    match value {
        Value::Object(map) => {
            let raw = map
                .get("value")
                .ok_or_else(|| PackError::InvalidNumber("missing value".into()))?;
            let unit = map
                .get("unit")
                .and_then(Value::as_str)
                .unwrap_or(default_unit);
            Ok((scalar_text(raw)?, unit.to_owned()))
        }
        Value::String(text) => {
            let trimmed = text.trim();
            for suffix in kind.text_suffixes() {
                if let Some(prefix) = strip_suffix_ignoring_case(trimmed, suffix)
                    && !prefix.trim().is_empty()
                {
                    return Ok((prefix.trim().to_owned(), (*suffix).to_owned()));
                }
            }
            Ok((trimmed.to_owned(), default_unit.to_owned()))
        }
        Value::Number(_) => Ok((scalar_text(value)?, default_unit.to_owned())),
        _ => Err(PackError::InvalidNumber(value.to_string())),
    }
}

/// `text` without a trailing `suffix` (an ASCII word) however the suffix is cased.
fn strip_suffix_ignoring_case<'a>(text: &'a str, suffix: &str) -> Option<&'a str> {
    let split = text.len().checked_sub(suffix.len())?;
    let tail = text.get(split..)?;
    tail.eq_ignore_ascii_case(suffix).then(|| &text[..split])
}

fn scalar_text(value: &Value) -> PackResult<String> {
    match value {
        Value::String(value) => Ok(value.clone()),
        Value::Number(value) => Ok(value.to_string()),
        _ => Err(PackError::InvalidNumber(value.to_string())),
    }
}

fn parse_rational(text: &str) -> PackResult<(i128, i128)> {
    let text = normalized(text);
    let invalid = || PackError::InvalidNumber(text.clone());
    let number = |digits: &str| digits.parse::<i128>().map_err(|_| invalid());
    let signed = |negative: bool, magnitude: i128| if negative { -magnitude } else { magnitude };
    match read_written(&text).ok_or_else(invalid)? {
        Written::Decimal {
            negative,
            whole,
            fraction,
        } => {
            let denominator = 10_i128
                .checked_pow(fraction.len() as u32)
                .ok_or_else(invalid)?;
            let fraction = if fraction.is_empty() {
                0
            } else {
                number(fraction)?
            };
            let magnitude = number(whole)?
                .checked_mul(denominator)
                .and_then(|scaled| scaled.checked_add(fraction))
                .ok_or_else(invalid)?;
            Ok((signed(negative, magnitude), denominator))
        }
        Written::Ratio {
            negative,
            numerator,
            denominator,
        } => match number(denominator)? {
            0 => Err(invalid()),
            denominator => Ok((signed(negative, number(numerator)?), denominator)),
        },
        Written::Mixed {
            negative,
            whole,
            numerator,
            denominator,
        } => {
            let denominator = number(denominator)?;
            if denominator == 0 {
                return Err(invalid());
            }
            let numerator = number(numerator)?;
            let magnitude = number(whole)?
                .checked_mul(denominator)
                .and_then(|scaled| scaled.checked_add(numerator))
                .ok_or_else(invalid)?;
            Ok((signed(negative, magnitude), denominator))
        }
    }
}

fn round_half_even(numerator: i128, denominator: i128) -> PackResult<i128> {
    if denominator <= 0 {
        return Err(PackError::InvalidNumber("zero denominator".into()));
    }
    let sign = if numerator < 0 { -1 } else { 1 };
    let absolute = numerator.abs();
    let quotient = absolute / denominator;
    let remainder = absolute % denominator;
    let rounded = match (remainder * 2).cmp(&denominator) {
        Ordering::Less => quotient,
        Ordering::Greater => quotient + 1,
        Ordering::Equal if quotient % 2 == 0 => quotient,
        Ordering::Equal => quotient + 1,
    };
    Ok(sign * rounded)
}

/// Renders `numerator / denominator` rounded to `places` digits, ties to even
/// (matches Python's `Decimal.quantize` default context, rather than truncating the
/// remainder as this used to).
fn decimal_string(numerator: i128, denominator: i128, places: usize) -> String {
    let negative = numerator < 0;
    let magnitude = numerator.unsigned_abs();
    let denominator = denominator.unsigned_abs();
    let mut whole = magnitude / denominator;
    let mut remainder = magnitude % denominator;
    let mut fraction = String::with_capacity(places);
    for _ in 0..places {
        remainder *= 10;
        fraction.push(char::from(b'0' + (remainder / denominator) as u8));
        remainder %= denominator;
    }
    let last_digit = fraction
        .as_bytes()
        .last()
        .map(|digit| (digit - b'0') % 2 == 1)
        .unwrap_or(whole % 2 == 1);
    let round_up = remainder * 2 > denominator || (remainder * 2 == denominator && last_digit);
    if round_up {
        match fraction.rfind(|digit: char| digit != '9') {
            Some(index) => {
                let bumped = char::from(fraction.as_bytes()[index] + 1);
                fraction.replace_range(index..=index, &bumped.to_string());
                fraction.replace_range(index + 1.., &"0".repeat(fraction.len() - index - 1));
            }
            None => {
                whole += 1;
                fraction = "0".repeat(fraction.len());
            }
        }
    }
    while fraction.ends_with('0') {
        fraction.pop();
    }
    let sign = if negative && (whole != 0 || !fraction.is_empty()) {
        "-"
    } else {
        ""
    };
    if fraction.is_empty() {
        format!("{sign}{whole}")
    } else {
        format!("{sign}{whole}.{fraction}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VAST: &str = "10000000000000000000000000000000000000000";
    const CEILING: u64 = (1 << 53) - 1;

    fn overflowed(text: &str) -> Overflowed {
        overflowed_measure(
            &Value::String(text.into()),
            "mm",
            MeasureKind::Length,
            CEILING,
        )
    }

    #[test]
    fn half_even_rounding_refuses_a_denominator_that_is_not_positive() {
        assert_eq!(round_half_even(5, 2).unwrap(), 2);
        assert_eq!(round_half_even(7, 2).unwrap(), 4);
        for denominator in [0, -2] {
            assert!(matches!(
                round_half_even(5, denominator),
                Err(PackError::InvalidNumber(message)) if message == "zero denominator"
            ));
        }
    }

    #[test]
    fn every_spelling_of_a_vast_length_is_past_the_ceiling() {
        for text in [
            VAST.to_owned(),
            format!("{VAST}.5"),
            format!("{VAST}/3"),
            format!("{VAST} 1/2"),
            format!("{VAST} cm"),
            format!("+{VAST}"),
        ] {
            assert_eq!(overflowed(&text), Overflowed::Beyond, "{text}");
        }
        let object = serde_json::json!({"value": VAST, "unit": "ticks"});
        assert_eq!(
            overflowed_measure(&object, "mm", MeasureKind::Length, CEILING),
            Overflowed::Beyond
        );
    }

    #[test]
    fn a_vast_negative_measure_is_negative() {
        assert_eq!(overflowed(&format!("-{VAST}")), Overflowed::Negative);
        assert_eq!(overflowed(&format!("-{VAST} 1/2")), Overflowed::Negative);
    }

    #[test]
    fn a_vanishing_or_unreadable_measure_is_neither() {
        for text in [
            format!("1/{VAST}"),
            format!("0/{VAST}"),
            format!("{VAST}/0"),
            format!("{VAST}x"),
            "-0".to_owned(),
        ] {
            assert_eq!(overflowed(&text), Overflowed::Unreadable, "{text}");
        }
        assert_eq!(
            overflowed_measure(&Value::Bool(true), "mm", MeasureKind::Length, CEILING),
            Overflowed::Unreadable
        );
        let furlongs = serde_json::json!({"value": VAST, "unit": "furlong"});
        assert_eq!(
            overflowed_measure(&furlongs, "mm", MeasureKind::Length, CEILING),
            Overflowed::Unreadable
        );
    }

    #[test]
    fn the_ceiling_itself_rounds_to_nearest_with_the_tie_to_even() {
        // MAX + 1/2 ticks rounds to MAX + 1, past the ceiling; MAX + 1/2 - 1/10 does not.
        let doubled = 2 * u128::from(CEILING) + 1;
        let tie = format!("{doubled}/2");
        let ticks = |text: &str| {
            overflowed_measure(
                &Value::String(text.into()),
                "ticks",
                MeasureKind::Length,
                CEILING,
            )
        };
        assert_eq!(ticks(&tie), Overflowed::Beyond);
        assert_eq!(
            ticks(&format!("{}/20", doubled * 10 - 2)),
            Overflowed::Unreadable
        );
        assert_eq!(digits_add(&[9, 9], &[1]), vec![0, 0, 1]);
    }

    #[test]
    fn each_unit_is_worth_its_definition_in_ticks() {
        // 1 in = 25.4 mm, 1 ft = 12 in, 1 lb = 453.59237 g, 1 oz = 1/16 lb.
        let one = |unit: &str| serde_json::json!({"value": "1", "unit": unit});
        for (unit, ticks) in [
            ("mm", 16_000),
            ("cm", 160_000),
            ("m", 16_000_000),
            ("in", 406_400),
            ("ft", 4_876_800),
            ("ticks", 1),
        ] {
            assert_eq!(Length::parse(&one(unit), "mm").unwrap().0, ticks, "{unit}");
        }
        for (unit, ticks) in [
            ("mg", 8_000),
            ("g", 8_000_000),
            ("kg", 8_000_000_000),
            ("oz", 226_796_185),
            ("lb", 3_628_738_960),
            ("ticks", 1),
        ] {
            assert_eq!(Weight::parse(&one(unit), "g").unwrap().0, ticks, "{unit}");
        }
    }

    #[test]
    fn fractional_inches_are_exact() {
        assert_eq!(
            Length::parse(&Value::String("12 3/8".into()), "in")
                .unwrap()
                .0,
            5_029_200
        );
    }

    #[test]
    fn pounds_are_exact() {
        assert_eq!(
            Weight::parse(&serde_json::json!({"value":"2","unit":"lb"}), "g")
                .unwrap()
                .0,
            7_257_477_920
        );
    }

    #[test]
    fn decimal_string_rounds_ties_to_even_matching_python() {
        // This used to truncate to "1.96849901"; Python's Decimal.quantize
        // (ROUND_HALF_EVEN, its default context) renders "1.96849902".
        assert_eq!(decimal_string(799_998, 406_400, 8), "1.96849902");
        assert_eq!(decimal_string(406_400, 16_000, 8), "25.4");
        assert_eq!(decimal_string(0, 16_000, 8), "0");
        assert_eq!(decimal_string(-406_400, 16_000, 8), "-25.4");
    }

    #[test]
    fn decimal_string_breaks_exact_ties_to_the_even_digit() {
        assert_eq!(decimal_string(1, 2, 0), "0");
        assert_eq!(decimal_string(3, 2, 0), "2");
        assert_eq!(decimal_string(5, 4, 1), "1.2");
        assert_eq!(decimal_string(7, 4, 1), "1.8");
    }

    #[test]
    fn decimal_string_carries_a_round_up_through_repeated_nines() {
        // 99999999/100000000 * 10^... rounds the trailing 9s all the way into the whole part.
        assert_eq!(decimal_string(1_999_999_999, 2_000_000_000, 8), "1");
    }
}
