//! Versioned carrier rating: tariffs, dimensional weight, surcharges and accessorials.
//!
//! An independent implementation of the same contract `docs/COMMERCE-API.md` specifies,
//! not a transliteration of the reference one. Every division rounds up and every
//! product is taken in `i128`, so a quote can neither wrap nor land a minor unit below
//! what the tariff charges.

use std::collections::BTreeMap;

/// The largest amount a quote may report: 2^53 - 1, the largest integer every engine and
/// every JSON reader holds exactly (the money ceiling, M2).
const MAX_QUOTE_AMOUNT: i128 = (1 << 53) - 1;

/// Exact `ceil(a * b / d)` for non-negative inputs, taken in `i128`: the product of two
/// 64-bit values cannot overflow it, so nothing wraps before `reported` looks at the result.
fn ceil_mul_div(a: i128, b: i128, d: i128) -> i128 {
    debug_assert!(d > 0 && a >= 0 && b >= 0);
    (a * b + d - 1) / d
}

/// An amount the quote reports, narrowed only once it is known to be at most the ceiling.
fn reported(amount: i128) -> Result<i64, RatingRejection> {
    if amount > MAX_QUOTE_AMOUNT {
        return Err(RatingRejection::AmountAboveCeiling(amount));
    }
    Ok(i64::try_from(amount).expect("an amount within 2^53 - 1 fits i64"))
}

/// One named accessorial: either a flat charge or a permille-of-base charge, never both.
#[derive(Debug, Clone)]
pub struct AccessorialCharge {
    pub accessorial_id: String,
    pub flat_charge_minor: Option<i64>,
    pub permille_of_base: Option<i64>,
}

impl AccessorialCharge {
    pub fn charge_minor(&self, base_charge_minor: i64) -> Result<i64, RatingRejection> {
        match (self.flat_charge_minor, self.permille_of_base) {
            (Some(flat), _) => reported(i128::from(flat)),
            (None, Some(permille)) => reported(ceil_mul_div(
                i128::from(base_charge_minor),
                i128::from(permille),
                1000,
            )),
            (None, None) => Ok(0),
        }
    }
}

/// One immutable, numbered version of one `(carrier_id, service_id)` pair's rate card.
#[derive(Debug, Clone)]
pub struct Tariff {
    pub carrier_id: String,
    pub service_id: String,
    pub version: i64,
    pub effective_at: i64,
    pub dimensional_weight_divisor: i64,
    pub cost_per_dimensional_kg_minor: BTreeMap<String, i64>,
    pub minimum_charge_minor: i64,
    pub fuel_surcharge_permille: i64,
    pub accessorials: BTreeMap<String, AccessorialCharge>,
}

/// What is being rated.
#[derive(Debug, Clone)]
pub struct RatingRequest {
    pub zone: String,
    pub actual_weight_g: i64,
    pub volume_mm3: i64,
    pub requested_accessorials: Vec<String>,
}

/// The fully itemized, auditable result of one rating call.
#[derive(Debug, Clone)]
pub struct RateBreakdown {
    pub carrier_id: String,
    pub service_id: String,
    pub tariff_version: i64,
    pub zone: String,
    pub actual_weight_g: i64,
    pub dimensional_weight_g: i64,
    pub billed_weight_g: i64,
    pub base_charge_minor: i64,
    pub minimum_charge_applied: bool,
    pub fuel_surcharge_minor: i64,
    pub accessorial_charges_minor: Vec<(String, i64)>,
    pub total_minor: i64,
}

/// A rejection the rating model is entitled to make, carried structurally so the API
/// boundary never has to parse a reason back out of prose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RatingRejection {
    TariffNotFound,
    NoEffectiveTariff,
    UnavailableZone(String),
    UnavailableAccessorials(Vec<String>),
    /// A reported amount, exactly computed, is above 2^53 - 1; the API boundary reports it
    /// as malformed input, since a double could not hold it.
    AmountAboveCeiling(i128),
}

/// Per-`(carrier_id, service_id)` append-only tariff history.
///
/// Complexity: `publish` is `O(1)`; every resolution is one `O(h)` scan of that pair's
/// history; rating one request is `O(a)` in requested accessorials.
#[derive(Debug, Default)]
pub struct CarrierRegistry {
    histories: BTreeMap<(String, String), Vec<Tariff>>,
}

impl CarrierRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a new, numbered version. Its number is its 1-based position in the history.
    pub fn publish(&mut self, mut tariff: Tariff) -> i64 {
        let key = (tariff.carrier_id.clone(), tariff.service_id.clone());
        let history = self.histories.entry(key).or_default();
        tariff.version = history.len() as i64 + 1;
        let version = tariff.version;
        history.push(tariff);
        version
    }

    pub fn has(&self, carrier_id: &str, service_id: &str) -> bool {
        self.histories
            .contains_key(&(carrier_id.to_owned(), service_id.to_owned()))
    }

    pub fn tariff(
        &self,
        carrier_id: &str,
        service_id: &str,
        version: i64,
    ) -> Result<&Tariff, RatingRejection> {
        self.history(carrier_id, service_id)?
            .iter()
            .find(|tariff| tariff.version == version)
            .ok_or(RatingRejection::TariffNotFound)
    }

    /// The version effective at `as_of`: the highest `effective_at` not after it, ties
    /// broken by the higher (later-published) version number.
    pub fn effective_tariff(
        &self,
        carrier_id: &str,
        service_id: &str,
        as_of: i64,
    ) -> Result<&Tariff, RatingRejection> {
        self.history(carrier_id, service_id)?
            .iter()
            .filter(|tariff| tariff.effective_at <= as_of)
            .max_by_key(|tariff| (tariff.effective_at, tariff.version))
            .ok_or(RatingRejection::NoEffectiveTariff)
    }

    fn history(&self, carrier_id: &str, service_id: &str) -> Result<&[Tariff], RatingRejection> {
        self.histories
            .get(&(carrier_id.to_owned(), service_id.to_owned()))
            .map(Vec::as_slice)
            .ok_or(RatingRejection::TariffNotFound)
    }
}

/// Rate a request against one already-resolved immutable tariff version.
pub fn rate_tariff(
    tariff: &Tariff,
    request: &RatingRequest,
) -> Result<RateBreakdown, RatingRejection> {
    if !tariff
        .cost_per_dimensional_kg_minor
        .contains_key(&request.zone)
    {
        return Err(RatingRejection::UnavailableZone(request.zone.clone()));
    }
    let mut unknown: Vec<String> = request
        .requested_accessorials
        .iter()
        .filter(|id| !tariff.accessorials.contains_key(*id))
        .cloned()
        .collect();
    if !unknown.is_empty() {
        unknown.sort();
        unknown.dedup();
        return Err(RatingRejection::UnavailableAccessorials(unknown));
    }

    // Reported amounts are checked in reporting order, so the first one past the ceiling is
    // the one named; each is computed exactly in `i128` first.
    // Dimensional weight in grams is volume (mm^3) over the divisor, rounded up.
    let dimensional_weight_g = reported(ceil_mul_div(
        i128::from(request.volume_mm3),
        1,
        i128::from(tariff.dimensional_weight_divisor),
    ))?;
    let billed_weight_g = reported(i128::from(
        request.actual_weight_g.max(dimensional_weight_g),
    ))?;

    let rate_per_kg = tariff.cost_per_dimensional_kg_minor[&request.zone];
    let raw_base_charge_minor =
        ceil_mul_div(i128::from(billed_weight_g), i128::from(rate_per_kg), 1000);
    let minimum_charge_applied = raw_base_charge_minor < i128::from(tariff.minimum_charge_minor);
    let base_charge_minor = reported(if minimum_charge_applied {
        i128::from(tariff.minimum_charge_minor)
    } else {
        raw_base_charge_minor
    })?;

    let fuel_surcharge_minor = reported(ceil_mul_div(
        i128::from(base_charge_minor),
        i128::from(tariff.fuel_surcharge_permille),
        1000,
    ))?;

    let accessorial_charges_minor: Vec<(String, i64)> = request
        .requested_accessorials
        .iter()
        .map(|id| {
            Ok((
                id.clone(),
                tariff.accessorials[id].charge_minor(base_charge_minor)?,
            ))
        })
        .collect::<Result<_, RatingRejection>>()?;
    let accessorial_total: i128 = accessorial_charges_minor
        .iter()
        .map(|(_, amount)| i128::from(*amount))
        .sum();
    let total_minor = reported(
        i128::from(base_charge_minor) + i128::from(fuel_surcharge_minor) + accessorial_total,
    )?;

    Ok(RateBreakdown {
        carrier_id: tariff.carrier_id.clone(),
        service_id: tariff.service_id.clone(),
        tariff_version: tariff.version,
        zone: request.zone.clone(),
        actual_weight_g: request.actual_weight_g,
        dimensional_weight_g,
        billed_weight_g,
        base_charge_minor,
        minimum_charge_applied,
        fuel_surcharge_minor,
        accessorial_charges_minor,
        total_minor,
    })
}
