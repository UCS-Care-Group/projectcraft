//! Costs, as Microsoft Project's "Enter costs for resources"
//! (<https://support.microsoft.com/en-us/project/enter-costs-for-resources>):
//!
//! - A work resource costs its **standard rate** per hour of work, plus its **cost/use** once per
//!   assignment. A material costs its standard rate per unit of its label (`$100` a ton), plus
//!   its cost/use. A cost resource costs the amount entered on each assignment, whatever the work.
//! - A task costs its assignments plus its own **fixed cost**. A summary task costs its own
//!   assignments and fixed cost plus its subtasks'; the project costs every task's own cost.
//! - Overtime rates and Accrue At are held for when actual work is tracked; they change no cost
//!   here.
//!
//! Money is whole minor units of the project's one [`Currency`] (pence for GBP), so no cost is a
//! float. An assignment's cost is worked out in `i128` and rounded half to even once, so the same
//! plan gives the same totals to the penny on every platform. Sums saturate instead of
//! overflowing; a total that large is far past [`MAX_MONEY`] anyway.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::resource::check_money;
use crate::{
    Assignment, AssignmentWork, DEFAULT_CURRENCY_SYMBOL, DEFAULT_MINOR_DIGITS, MAX_MONEY, ModelError, Outline, Project, Resource, TaskId, Units,
    assignment_work, format_money, parse_money,
};

/// Most decimal places a currency's minor unit has (ISO 4217 lists none with more than four).
pub const MAX_CURRENCY_DIGITS: u32 = 4;
/// Longest currency symbol, in characters (`CHF`, `A$`, `KD`).
pub const MAX_CURRENCY_SYMBOL: usize = 8;
/// The default currency's ISO 4217 code.
pub const DEFAULT_CURRENCY_CODE: &str = "GBP";

/// Common currencies: ISO 4217 code, the symbol shown, and the decimal places of the minor
/// unit. Any other three-letter code can be used with its symbol and places given.
pub const KNOWN_CURRENCIES: [(&str, &str, u32); 16] = [
    ("GBP", "£", 2),
    ("EUR", "€", 2),
    ("USD", "$", 2),
    ("AUD", "A$", 2),
    ("CAD", "C$", 2),
    ("NZD", "NZ$", 2),
    ("CHF", "CHF", 2),
    ("SEK", "kr", 2),
    ("NOK", "kr", 2),
    ("DKK", "kr", 2),
    ("INR", "₹", 2),
    ("ZAR", "R", 2),
    ("JPY", "¥", 0),
    ("KRW", "₩", 0),
    ("BHD", "BD", 3),
    ("KWD", "KD", 3),
];

/// The project's one currency: its ISO 4217 code, the symbol amounts show with, and how many
/// decimal places its minor unit has (2 for pence, 0 for yen, 3 for fils). There are no exchange
/// rates: changing the currency relabels amounts, never converts them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Currency {
    pub code: String,
    pub symbol: String,
    pub digits: u32,
}

impl Default for Currency {
    fn default() -> Self {
        Currency { code: DEFAULT_CURRENCY_CODE.into(), symbol: DEFAULT_CURRENCY_SYMBOL.into(), digits: DEFAULT_MINOR_DIGITS }
    }
}

impl Currency {
    /// A currency by its code (any case), with the symbol and decimal places given, or else the
    /// known ones for that code ([`KNOWN_CURRENCIES`]); an unknown code shows as itself with two
    /// places. Refused when the code isn't three letters, the symbol is too long or could be
    /// mistaken for part of an amount, or the places are more than [`MAX_CURRENCY_DIGITS`].
    pub fn new(code: &str, symbol: Option<&str>, digits: Option<u32>) -> Result<Currency, ModelError> {
        let code = code.trim().to_ascii_uppercase();
        let known = KNOWN_CURRENCIES.iter().find(|(c, _, _)| *c == code);
        let c = Currency {
            symbol: symbol.map(|s| s.trim().to_owned()).or_else(|| known.map(|k| k.1.to_owned())).unwrap_or_else(|| code.clone()),
            digits: digits.or(known.map(|k| k.2)).unwrap_or(DEFAULT_MINOR_DIGITS),
            code,
        };
        c.check()?;
        Ok(c)
    }

    /// Refuses a currency no command could have made: for a loaded file.
    pub fn check(&self) -> Result<(), ModelError> {
        if self.code.len() != 3 || !self.code.bytes().all(|b| b.is_ascii_uppercase()) {
            let code: String = self.code.chars().take(20).collect();
            return Err(ModelError::Invalid(format!("`{code}` is not a currency code (three letters, such as GBP)")));
        }
        let amount_like = |c: char| c.is_ascii_digit() || c.is_control() || matches!(c, '.' | ',' | '-' | '+' | '/' | '[' | ']');
        if self.symbol.chars().count() > MAX_CURRENCY_SYMBOL || self.symbol.trim() != self.symbol || self.symbol.contains(amount_like) {
            return Err(ModelError::Invalid(format!(
                "a currency symbol is at most {MAX_CURRENCY_SYMBOL} characters, without digits, signs, brackets, commas, full stops or spaces at its ends"
            )));
        }
        if self.digits > MAX_CURRENCY_DIGITS {
            return Err(ModelError::Invalid(format!("a currency has 0 to {MAX_CURRENCY_DIGITS} decimal places")));
        }
        Ok(())
    }

    pub fn is_default(&self) -> bool {
        *self == Currency::default()
    }

    /// An amount as the sheet shows it: `£1,234.50`, `¥1,235`, `BD1.250`.
    pub fn format(&self, minor: i64) -> String {
        format_money(minor, &self.symbol, self.digits)
    }

    /// An amount as typed: an optional symbol, digits with optional thousands commas, up to the
    /// currency's decimal places; whole minor units.
    pub fn parse(&self, text: &str) -> Result<i64, ModelError> {
        parse_money(text, &self.symbol, self.digits)
    }
}

/// `n ÷ d` rounded to the nearest whole number, a half to the even one (banker's rounding), so
/// halves don't all go the same way. `d` below 1 counts as 1.
pub fn div_half_even(n: i128, d: i128) -> i128 {
    let d = d.max(1);
    let (q, r) = (n.div_euclid(d), n.rem_euclid(d));
    // r < d ≤ i128::MAX / 2 for every caller here, so doubling it can't overflow; saturate anyway.
    match r.saturating_mul(2).cmp(&d) {
        std::cmp::Ordering::Less => q,
        std::cmp::Ordering::Greater => q.saturating_add(1),
        std::cmp::Ordering::Equal if q % 2 == 0 => q,
        std::cmp::Ordering::Equal => q.saturating_add(1),
    }
}

fn saturate(n: i128) -> i64 {
    i64::try_from(n).unwrap_or(if n < 0 { i64::MIN } else { i64::MAX })
}

/// What one assignment costs, in minor units: a work resource's hours × its standard rate, a
/// material's quantity × its standard rate (each rounded half to even once), plus its cost/use;
/// a cost resource's amount. `work` is what [`assignment_work`] says the assignment uses.
pub fn assignment_cost(units: Units, work: AssignmentWork, resource: &Resource) -> i64 {
    let rate = i128::from(resource.standard_rate);
    let per_use = i128::from(resource.cost_per_use);
    let n = match (units, work) {
        (Units::Cost(amount), _) => i128::from(amount),
        // Minutes × a rate per hour; quantities are in hundredths of the label.
        (_, AssignmentWork::Minutes(m)) => div_half_even(i128::from(m).saturating_mul(rate), 60).saturating_add(per_use),
        (_, AssignmentWork::Quantity(q)) => div_half_even(i128::from(q).saturating_mul(rate), 100).saturating_add(per_use),
        (_, AssignmentWork::None) => per_use,
    };
    saturate(n)
}

/// Each task's own cost and, for a summary, its subtasks' too: the same roll-up the Work column
/// uses, over every task (a task with nothing costs 0).
fn roll_up(project: &Project, own: &BTreeMap<TaskId, i64>) -> BTreeMap<TaskId, i64> {
    let outline = Outline::of(project);
    let mut total: BTreeMap<TaskId, i64> = BTreeMap::new();
    // The open summaries above the current row.
    let mut open: Vec<(usize, TaskId)> = Vec::new();
    for (row, t) in project.tasks.iter().enumerate() {
        while open.last().is_some_and(|(r, _)| outline.subtree_end(*r) <= row) {
            open.pop();
        }
        let c = own.get(&t.id).copied().unwrap_or(0);
        for id in std::iter::once(t.id).chain(open.iter().map(|(_, id)| *id)) {
            let sum = total.entry(id).or_insert(0);
            *sum = sum.saturating_add(c);
        }
        if outline.is_summary(row) {
            open.push((row, t.id));
        }
    }
    total
}

impl Project {
    /// What one assignment costs ([`assignment_cost`]), its work read for its task's duration; 0
    /// when its task or resource is missing (only a damaged file could hold that).
    pub fn assignment_cost(&self, a: &Assignment) -> i64 {
        let (Ok(t), Ok(r)) = (self.task(a.task), self.resource(a.resource)) else { return 0 };
        assignment_cost(a.units, assignment_work(a.units, t.duration, &self.options), r)
    }

    /// Each task's own cost: its assignments' plus its fixed cost, not its subtasks'.
    fn own_costs(&self) -> BTreeMap<TaskId, i64> {
        let mut own: BTreeMap<TaskId, i64> = self.tasks.iter().map(|t| (t.id, t.fixed_cost)).collect();
        for a in &self.assignments {
            if let Some(c) = own.get_mut(&a.task) {
                *c = c.saturating_add(self.assignment_cost(a));
            }
        }
        own
    }

    /// The Cost column: each task's cost, its assignments' plus its fixed cost, and for a summary
    /// its subtasks' as well. Every task is in it.
    pub fn cost_by_task(&self) -> BTreeMap<TaskId, i64> {
        roll_up(self, &self.own_costs())
    }

    /// The project's total cost: every task's own cost, once.
    pub fn total_cost(&self) -> i64 {
        self.own_costs().values().fold(0i64, |sum, c| sum.saturating_add(*c))
    }

    /// Sets a task's fixed cost, in minor units; summaries take one too, as in Project.
    pub fn set_fixed_cost(&mut self, id: TaskId, minor: i64) -> Result<(), ModelError> {
        let minor = check_money(minor)?;
        self.task_mut(id)?.fixed_cost = minor;
        Ok(())
    }

    /// Changes the project's currency. Amounts keep their value in the new currency's units
    /// (`£12.50` becomes `€12.50`; there are no exchange rates). With fewer decimal places they
    /// round half to even (`£12.50` becomes `¥12`); with more they gain zeros, and are refused
    /// if any would pass [`MAX_MONEY`]. All or nothing.
    pub fn set_currency(&mut self, currency: Currency) -> Result<(), ModelError> {
        currency.check()?;
        let (from, to) = (self.currency.digits, currency.digits);
        let mut p = self.clone();
        if from != to {
            let scale = |n: i64| -> Result<i64, ModelError> {
                let n = i128::from(n);
                let moved = if to > from {
                    n.checked_mul(10i128.pow(to.saturating_sub(from).min(MAX_CURRENCY_DIGITS))).unwrap_or(i128::MAX)
                } else {
                    div_half_even(n, 10i128.pow(from.saturating_sub(to).min(MAX_CURRENCY_DIGITS)))
                };
                let moved = i64::try_from(moved).unwrap_or(i64::MAX);
                check_money(moved).map_err(|_| {
                    ModelError::Invalid(format!("an amount would pass the largest one allowed ({}) in {}", currency.format(MAX_MONEY), currency.code))
                })
            };
            for r in &mut p.resources {
                r.standard_rate = scale(r.standard_rate)?;
                r.overtime_rate = scale(r.overtime_rate)?;
                r.cost_per_use = scale(r.cost_per_use)?;
            }
            for a in &mut p.assignments {
                if let Units::Cost(m) = a.units {
                    a.units = Units::Cost(scale(m)?);
                }
            }
            for t in &mut p.tasks {
                t.fixed_cost = scale(t.fixed_cost)?;
            }
        }
        p.currency = currency;
        *self = p;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Date, DurationUnit, ResourceField, ResourceId, ResourceType};

    /// A day of duration at the default 8 hours.
    const D: i64 = 480;

    fn project() -> Project {
        Project::new("t", Date::from_ymd(2026, 1, 5).unwrap())
    }

    fn work(p: &mut Project, name: &str, rate: i64, per_use: i64) -> ResourceId {
        let id = p.add_resource(Some(name), ResourceType::Work, None).unwrap();
        p.set_resource(id, &[ResourceField::StandardRate(rate), ResourceField::CostPerUse(per_use)]).unwrap();
        id
    }

    #[test]
    fn half_to_even() {
        for (n, d, q) in
            [(5, 10, 0), (15, 10, 2), (25, 10, 2), (35, 10, 4), (14, 10, 1), (16, 10, 2), (-5, 10, 0), (-15, 10, -2), (-16, 10, -2), (7, 0, 7)]
        {
            assert_eq!(div_half_even(n, d), q, "{n}/{d}");
        }
    }

    /// Microsoft's "Enter costs for resources": a person's standard rate is per hour ($25.00/hr),
    /// a fee per assignment goes in Cost/Use, a material's standard rate is per unit of its label
    /// (cement at $100 a ton), a cost resource carries its own amount ($500) whatever the work,
    /// and a task can take a fixed cost of its own. In US dollars, to the cent.
    /// <https://support.microsoft.com/en-us/project/enter-costs-for-resources>
    #[test]
    fn enter_costs_for_resources_worked_example() {
        let mut p = project();
        p.set_currency(Currency::new("usd", None, None).unwrap()).unwrap();
        let pour = p.add_task("Pour slab", 5 * D).unwrap();
        let ann = work(&mut p, "Ann", 2500, 0);
        let crane = work(&mut p, "Crane", 8000, 15_000);
        let cement = p.add_resource(Some("Cement"), ResourceType::Material, None).unwrap();
        p.set_resource(cement, &[ResourceField::MaterialLabel("ton".into()), ResourceField::StandardRate(10_000)]).unwrap();
        let travel = p.add_resource(Some("Travel"), ResourceType::Cost, None).unwrap();
        p.set_resource_names(pour, "Ann, Crane[50%], Cement[3 ton], Travel[$500]").unwrap();
        let cost = |p: &Project, r: ResourceId| p.assignment_cost(p.assignments_of(pour).find(|a| a.resource == r).unwrap());
        // Ann: 5 days × 8h × 100% = 40h × $25.00 = $1,000.00.
        assert_eq!(cost(&p, ann), 100_000);
        // The crane: 40h × 50% = 20h × $80.00 = $1,600.00, plus $150.00 per use.
        assert_eq!(cost(&p, crane), 175_000);
        // Cement: 3 tons × $100.00 = $300.00. Travel: $500.00, whatever the duration.
        assert_eq!(cost(&p, cement), 30_000);
        assert_eq!(cost(&p, travel), 50_000);
        p.set_fixed_cost(pour, 25_000).unwrap();
        assert_eq!(p.cost_by_task().get(&pour), Some(&380_000));
        assert_eq!(p.currency.format(p.total_cost()), "$3,800.00");
        // Twice the duration: twice the work and its cost; the cement, travel and fee stay.
        p.task_mut(pour).unwrap().duration = 10 * D;
        assert_eq!(p.total_cost(), 200_000 + 335_000 + 30_000 + 50_000 + 25_000);
        // Overtime rates and Accrue At change nothing until work is tracked.
        let before = p.total_cost();
        p.set_resource(ann, &[ResourceField::OvertimeRate(9900), ResourceField::AccrueAt(crate::AccrueAt::End)]).unwrap();
        assert_eq!(p.total_cost(), before);
    }

    /// Costs round once per assignment, half to even: 20 minutes at £0.75/hr is 25p exactly; 10
    /// minutes is 12.5p, which goes to 12p; 30 minutes at £0.05/hr is 2.5p, which goes to 2p; 90
    /// minutes at £0.05/hr is 7.5p, which goes to 8p. Summing rounded assignments, never
    /// rounding the sum again.
    #[test]
    fn each_assignment_rounds_half_to_even_once() {
        let mut p = project();
        let r = work(&mut p, "Ann", 75, 0);
        let cheap = work(&mut p, "Bob", 5, 0);
        let res = |p: &Project, id| p.resource(id).unwrap().clone();
        for (minutes, who, pence) in [(20, r, 25), (10, r, 12), (30, cheap, 2), (90, cheap, 8), (0, r, 0)] {
            assert_eq!(assignment_cost(Units::Work(10_000), AssignmentWork::Minutes(minutes), &res(&p, who)), pence, "{minutes}m");
        }
        // Two tasks of 10 minutes each at 75p/hr: 12p + 12p = 24p, not 25p (rounding the sum).
        for _ in 0..2 {
            let t = p.add_task("t", 10).unwrap();
            p.assign(t, r, None).unwrap();
        }
        assert_eq!(p.total_cost(), 24);
        // A variable material: 2.5 boxes/h over 3 hours is 7.5 boxes at £0.33 = 247.5p, to 248p.
        let boxes = p.add_resource(Some("Boxes"), ResourceType::Material, None).unwrap();
        p.set_resource(boxes, &[ResourceField::StandardRate(33)]).unwrap();
        let units = Units::Rate { quantity: 250, per: DurationUnit::Hours };
        let used = assignment_work(units, 180, &p.options);
        assert_eq!(assignment_cost(units, used, &res(&p, boxes)), 248);
    }

    /// A summary costs its own assignments and fixed cost plus its subtasks'; the project counts
    /// each task once.
    #[test]
    fn costs_roll_up_to_summaries_and_the_project() {
        let mut p = project();
        let ann = work(&mut p, "Ann", 1000, 0);
        let phase = p.add_task("Phase", D).unwrap();
        let a = p.add_task("A", D).unwrap();
        let b = p.add_task("B", 2 * D).unwrap();
        let after = p.add_task("After", D).unwrap();
        p.indent(&[a, b]).unwrap();
        p.assign(a, ann, None).unwrap();
        p.assign(b, ann, None).unwrap();
        p.set_fixed_cost(phase, 100).unwrap();
        p.set_fixed_cost(after, 200).unwrap();
        let c = p.cost_by_task();
        assert_eq!((c[&a], c[&b], c[&after]), (8000, 16_000, 200));
        assert_eq!(c[&phase], 100 + 8000 + 16_000);
        assert_eq!(p.total_cost(), 24_300);
        assert_eq!(c.len(), 4, "every task, even one that costs nothing");
        assert!(p.set_fixed_cost(after, -1).is_err());
        assert!(p.set_fixed_cost(after, MAX_MONEY + 1).is_err());
        assert!(p.set_fixed_cost(TaskId(99), 1).is_err());
    }

    /// Currencies with no minor unit (yen) and with three decimal places (Bahraini dinar).
    #[test]
    fn currencies_with_zero_and_three_decimal_places() {
        let jpy = Currency::new("JPY", None, None).unwrap();
        assert_eq!((jpy.symbol.as_str(), jpy.digits), ("¥", 0));
        assert_eq!(jpy.format(1_234_567), "¥1,234,567");
        assert_eq!(jpy.parse("¥1,500").unwrap(), 1_500);
        assert!(jpy.parse("¥1.5").is_err(), "yen have no decimal places");
        let bhd = Currency::new("bhd", None, None).unwrap();
        assert_eq!(bhd.format(1_250), "BD1.250");
        assert_eq!(bhd.parse("BD2.005").unwrap(), 2_005);
        assert!(bhd.parse("2.0005").is_err());
        // Rates per hour in fils: 20 minutes at BD1.001/hr is 333.67 fils, to 334.
        let mut p = project();
        p.set_currency(bhd).unwrap();
        let ann = work(&mut p, "Ann", 1_001, 0);
        let t = p.add_task("t", 20).unwrap();
        p.assign(t, ann, None).unwrap();
        assert_eq!(p.total_cost(), 334);
        assert_eq!(p.units_text_full(p.assignments_of(t).next().unwrap()), "100%");
        // An unknown code shows as itself, with two places unless told otherwise.
        let xts = Currency::new("XTS", None, Some(4)).unwrap();
        assert_eq!(xts.format(12_345), "XTS1.2345");
        for (code, symbol, digits) in [
            ("GB", None, None),
            ("GBPX", None, None),
            ("G1P", None, None),
            ("GBP", Some("1£"), None),
            ("GBP", Some("£."), None),
            ("GBP", Some("ABCDEFGHI"), None),
            ("GBP", None, Some(5)),
        ] {
            assert!(Currency::new(code, symbol, digits).is_err(), "{code} {symbol:?} {digits:?}");
        }
        assert_eq!(Currency::new("gbp", None, None).unwrap(), Currency::default());
    }

    /// Changing the currency relabels amounts: the same value in the new currency's units, rounded
    /// half to even when it has fewer places; refused, changing nothing, if an amount would get
    /// too large.
    #[test]
    fn changing_the_currency_keeps_the_value_shown() {
        let mut p = project();
        let ann = work(&mut p, "Ann", 1250, 325);
        let travel = p.add_resource(Some("Travel"), ResourceType::Cost, None).unwrap();
        let t = p.add_task("t", D).unwrap();
        p.assign(t, travel, Some(Units::Cost(15_050))).unwrap();
        p.set_fixed_cost(t, 9999).unwrap();
        p.set_currency(Currency::new("EUR", None, None).unwrap()).unwrap();
        assert_eq!(p.resource(ann).unwrap().standard_rate, 1250);
        assert_eq!(p.resource_names(t), "Travel[€150.50]");
        p.set_currency(Currency::new("JPY", None, None).unwrap()).unwrap();
        let r = p.resource(ann).unwrap();
        // 12.50 → 12 and 3.25 → 3 (half to even, and below a half); 150.50 → 150; 99.99 → 100.
        assert_eq!((r.standard_rate, r.cost_per_use, p.task(t).unwrap().fixed_cost), (12, 3, 100));
        assert_eq!(p.resource_names(t), "Travel[¥150]");
        p.set_currency(Currency::new("BHD", None, None).unwrap()).unwrap();
        assert_eq!(p.resource(ann).unwrap().standard_rate, 12_000);
        // Too large in the new units: refused, nothing changed.
        p.set_fixed_cost(t, MAX_MONEY / 10 + 1).unwrap();
        let before = p.clone();
        assert!(p.set_currency(Currency::new("XTS", None, Some(4)).unwrap()).is_err());
        assert_eq!(p, before);
        let bad = Currency { code: "x".into(), ..Currency::default() };
        assert!(p.set_currency(bad).is_err());
        assert_eq!(p, before);
    }

    /// Hostile amounts: the largest rates, units, durations and fees never overflow or panic; the
    /// totals saturate.
    #[test]
    fn the_largest_amounts_never_overflow() {
        let mut p = project();
        let big = work(&mut p, "Big", MAX_MONEY, MAX_MONEY);
        p.set_resource(big, &[ResourceField::MaxUnits(crate::MAX_UNITS)]).unwrap();
        let cost = p.add_resource(Some("Fee"), ResourceType::Cost, None).unwrap();
        let mat = p.add_resource(Some("Mat"), ResourceType::Material, None).unwrap();
        p.set_resource(mat, &[ResourceField::StandardRate(MAX_MONEY), ResourceField::CostPerUse(MAX_MONEY)]).unwrap();
        for _ in 0..3 {
            let t = p.add_task("t", crate::MAX_MINUTES).unwrap();
            p.set_fixed_cost(t, MAX_MONEY).unwrap();
            p.assign(t, big, Some(Units::Work(crate::MAX_UNITS))).unwrap();
            p.assign(t, cost, Some(Units::Cost(MAX_MONEY))).unwrap();
            p.assign(t, mat, Some(Units::Rate { quantity: crate::MAX_QUANTITY, per: DurationUnit::Minutes })).unwrap();
        }
        assert_eq!(p.total_cost(), i64::MAX);
        assert!(p.cost_by_task().values().all(|c| *c == i64::MAX));
        let _ = p.currency.format(i64::MAX);
        let _ = p.currency.format(i64::MIN);
    }

    proptest::proptest! {
        /// Any currency, any amount: formatting and reading back agree, and never panic.
        #[test]
        fn amounts_round_trip_in_any_currency(digits in 0u32..=MAX_CURRENCY_DIGITS, n in 0i64..=MAX_MONEY, code in "\\PC{0,5}", symbol in "\\PC{0,9}") {
            let c = Currency::new("XTS", None, Some(digits)).unwrap();
            proptest::prop_assert_eq!(c.parse(&c.format(n)).ok(), Some(n));
            if let Ok(c) = Currency::new(&code, Some(&symbol), Some(digits)) {
                proptest::prop_assert_eq!(c.parse(&c.format(n)).ok(), Some(n));
            }
        }

        #[test]
        fn half_even_is_within_a_half(n in proptest::num::i64::ANY, d in 1i64..=1_000_000) {
            let q = div_half_even(i128::from(n), i128::from(d));
            let diff = (q * i128::from(d) - i128::from(n)).abs() * 2;
            proptest::prop_assert!(diff <= i128::from(d));
        }
    }
}
