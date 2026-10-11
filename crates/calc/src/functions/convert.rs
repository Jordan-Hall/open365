//! `CONVERT`: unit conversion with the unit names, SI and binary prefixes of Excel. The factors were measured with
//! `CONVERT(1, unit, base)`; units that Excel keeps as "per base" constants (the `true` flag) are applied by division,
//! which reproduces its last digits on 91% of random conversions (see docs/testing/calc-coverage.md).

use super::{Res, arity, num_arg, text_arg};
use crate::eval::{Arg, EvalContext};
use crate::value::{ErrorKind, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cat {
    Mass,
    Distance,
    Time,
    Pressure,
    Force,
    Energy,
    Power,
    Magnetism,
    Temperature,
    Volume,
    Area,
    Information,
    Speed,
}

/// `(name, category, divide, constant, accepts prefixes)`.
static UNITS: &[(&str, Cat, bool, f64, bool)] = &[
    ("g", Cat::Mass, false, 1.0, true),
    ("sg", Cat::Mass, false, 14593.902937206363, false),
    ("lbm", Cat::Mass, false, 453.59237, false),
    ("u", Cat::Mass, false, 1.660538782e-24, true),
    ("ozm", Cat::Mass, false, 28.349523125, false),
    ("grain", Cat::Mass, false, 0.06479891, false),
    ("cwt", Cat::Mass, false, 45359.237, false),
    ("shweight", Cat::Mass, false, 45359.237, false),
    ("uk_cwt", Cat::Mass, false, 50802.34544, false),
    ("lcwt", Cat::Mass, false, 50802.34544, false),
    ("hweight", Cat::Mass, false, 50802.34544, false),
    ("stone", Cat::Mass, false, 6350.29318, false),
    ("ton", Cat::Mass, false, 907184.74, false),
    ("uk_ton", Cat::Mass, false, 1016046.9088, false),
    ("LTON", Cat::Mass, false, 1016046.9088, false),
    ("brton", Cat::Mass, false, 1016046.9088, false),
    ("m", Cat::Distance, false, 1.0, true),
    ("mi", Cat::Distance, false, 1609.344, false),
    ("Nmi", Cat::Distance, true, 0.0005399568034557236, false),
    ("in", Cat::Distance, true, 39.37007874015748, false),
    ("ft", Cat::Distance, true, 3.2808398950131235, false),
    ("yd", Cat::Distance, false, 0.9144, false),
    ("ang", Cat::Distance, true, 10000000000.0, true),
    ("ell", Cat::Distance, true, 0.8748906386701663, false),
    ("ly", Cat::Distance, false, 9460730472580800.0, true),
    ("parsec", Cat::Distance, false, 3.0856775812815532e+16, true),
    ("pc", Cat::Distance, false, 3.0856775812815532e+16, true),
    ("Pica", Cat::Distance, false, 0.0003527777777777778, false),
    ("Picapt", Cat::Distance, true, 2834.6456692913384, false),
    ("pica", Cat::Distance, true, 236.22047244094486, false),
    ("survey_mi", Cat::Distance, false, 1609.3472186944373, false),
    ("yr", Cat::Time, false, 31557600.0, false),
    ("day", Cat::Time, false, 86400.0, false),
    ("d", Cat::Time, false, 86400.0, false),
    ("hr", Cat::Time, false, 3600.0, false),
    ("mn", Cat::Time, false, 60.0, false),
    ("min", Cat::Time, false, 60.0, false),
    ("sec", Cat::Time, false, 1.0, true),
    ("s", Cat::Time, false, 1.0, true),
    ("Pa", Cat::Pressure, false, 1.0, true),
    ("p", Cat::Pressure, false, 1.0, true),
    ("atm", Cat::Pressure, false, 101325.0, true),
    ("at", Cat::Pressure, false, 101325.0, true),
    ("mmHg", Cat::Pressure, false, 133.322, true),
    ("psi", Cat::Pressure, false, 6894.757293168362, false),
    ("Torr", Cat::Pressure, false, 133.32236842105263, false),
    ("N", Cat::Force, false, 1.0, true),
    ("dyn", Cat::Force, true, 100000.0, true),
    ("dy", Cat::Force, true, 100000.0, true),
    ("lbf", Cat::Force, true, 0.22480894309971047, false),
    ("pond", Cat::Force, false, 0.00980665, true),
    ("J", Cat::Energy, false, 1.0, true),
    ("e", Cat::Energy, false, 1e-07, true),
    ("c", Cat::Energy, true, 0.23900573613766732, true),
    ("cal", Cat::Energy, true, 0.23884589662749595, true),
    ("eV", Cat::Energy, false, 1.602176487e-19, true),
    ("ev", Cat::Energy, false, 1.602176487e-19, true),
    ("HPh", Cat::Energy, false, 2684519.537696173, false),
    ("hh", Cat::Energy, true, 3.725061359986188e-07, false),
    ("Wh", Cat::Energy, true, 0.0002777777777777778, true),
    ("wh", Cat::Energy, false, 3600.0, true),
    ("flb", Cat::Energy, true, 0.7375621492772654, false),
    ("BTU", Cat::Energy, true, 0.0009478171203133171, false),
    ("btu", Cat::Energy, true, 0.0009478171203133171, false),
    ("HP", Cat::Power, false, 745.6998715822702, false),
    ("h", Cat::Power, false, 745.6998715822702, false),
    ("PS", Cat::Power, false, 735.49875, false),
    ("W", Cat::Power, false, 1.0, true),
    ("w", Cat::Power, false, 1.0, true),
    ("T", Cat::Magnetism, false, 1.0, true),
    ("ga", Cat::Magnetism, true, 10000.0, true),
    ("tsp", Cat::Volume, false, 0.00492892159375, false),
    ("tspm", Cat::Volume, false, 0.005, false),
    ("tbs", Cat::Volume, false, 0.01478676478125, false),
    ("oz", Cat::Volume, false, 0.0295735295625, false),
    ("cup", Cat::Volume, false, 0.2365882365, false),
    ("pt", Cat::Volume, false, 0.473176473, false),
    ("us_pt", Cat::Volume, false, 0.473176473, false),
    ("uk_pt", Cat::Volume, false, 0.56826125, true),
    ("qt", Cat::Volume, false, 0.946352946, false),
    ("uk_qt", Cat::Volume, false, 1.1365225, false),
    ("gal", Cat::Volume, false, 3.785411784, false),
    ("uk_gal", Cat::Volume, false, 4.54609, false),
    ("l", Cat::Volume, false, 1.0, true),
    ("L", Cat::Volume, false, 1.0, true),
    ("lt", Cat::Volume, false, 1.0, true),
    ("ang3", Cat::Volume, false, 1e-27, true),
    ("ang^3", Cat::Volume, false, 1e-27, true),
    ("barrel", Cat::Volume, false, 158.987294928, false),
    ("bushel", Cat::Volume, false, 35.23907016688, false),
    ("ft3", Cat::Volume, false, 28.316846592, false),
    ("ft^3", Cat::Volume, false, 28.316846592, false),
    ("in3", Cat::Volume, false, 0.016387064, false),
    ("in^3", Cat::Volume, false, 0.016387064, false),
    ("ly3", Cat::Volume, false, 8.467866646237152e+50, false),
    ("ly^3", Cat::Volume, false, 8.467866646237152e+50, false),
    ("m3", Cat::Volume, false, 1000.0, true),
    ("m^3", Cat::Volume, false, 1000.0, true),
    ("mi3", Cat::Volume, false, 4168181825440.5796, false),
    ("mi^3", Cat::Volume, false, 4168181825440.5796, false),
    ("yd3", Cat::Volume, false, 764.554857984, false),
    ("yd^3", Cat::Volume, false, 764.554857984, false),
    ("Nmi3", Cat::Volume, false, 6352182208000.0, false),
    ("Nmi^3", Cat::Volume, false, 6352182208000.0, false),
    ("Pica3", Cat::Volume, false, 4.39039566186557e-08, false),
    ("Pica^3", Cat::Volume, false, 4.39039566186557e-08, false),
    ("Picapt3", Cat::Volume, false, 4.39039566186557e-08, false),
    ("Picapt^3", Cat::Volume, false, 4.39039566186557e-08, false),
    ("GRT", Cat::Volume, false, 2831.6846592, false),
    ("regton", Cat::Volume, false, 2831.6846592, false),
    ("MTON", Cat::Volume, false, 1132.67386368, false),
    ("uk_acre", Cat::Area, false, 4046.8564224, false),
    ("us_acre", Cat::Area, true, 0.0002471043930466279, false),
    ("ang2", Cat::Area, true, 1e+20, true),
    ("ang^2", Cat::Area, true, 1e+20, true),
    ("ar", Cat::Area, false, 100.0, true),
    ("ft2", Cat::Area, true, 10.763910416709722, false),
    ("ft^2", Cat::Area, true, 10.763910416709722, false),
    ("ha", Cat::Area, false, 10000.0, false),
    ("in2", Cat::Area, false, 0.00064516, false),
    ("in^2", Cat::Area, false, 0.00064516, false),
    ("ly2", Cat::Area, true, 1.117250763128733e-32, false),
    ("ly^2", Cat::Area, true, 1.1172507631287331e-32, false),
    ("m2", Cat::Area, false, 1.0, true),
    ("m^2", Cat::Area, false, 1.0, true),
    ("Morgen", Cat::Area, false, 2500.0, false),
    ("mi2", Cat::Area, false, 2589988.110336, false),
    ("mi^2", Cat::Area, true, 3.8610215854244587e-07, false),
    ("Nmi2", Cat::Area, false, 3429904.0, false),
    ("Nmi^2", Cat::Area, true, 2.9155334959812285e-07, false),
    ("Pica2", Cat::Area, false, 1.2445216049382715e-07, false),
    ("Pica^2", Cat::Area, false, 1.2445216049382715e-07, false),
    ("Picapt2", Cat::Area, false, 1.2445216049382715e-07, false),
    ("Picapt^2", Cat::Area, true, 8035216.0704321405, false),
    ("yd2", Cat::Area, false, 0.83612736, false),
    ("yd^2", Cat::Area, false, 0.83612736, false),
    ("bit", Cat::Information, false, 1.0, true),
    ("byte", Cat::Information, false, 8.0, true),
    ("admkn", Cat::Speed, false, 0.5147733333333333, false),
    ("kn", Cat::Speed, false, 0.5144444444444445, false),
    ("m/h", Cat::Speed, false, 0.0002777777777777778, true),
    ("m/hr", Cat::Speed, false, 0.0002777777777777778, true),
    ("m/s", Cat::Speed, false, 1.0, true),
    ("m/sec", Cat::Speed, false, 1.0, true),
    ("mph", Cat::Speed, false, 0.44704, true),
    ("K", Cat::Temperature, false, 1.0, true),
    ("kel", Cat::Temperature, false, 1.0, true),
    ("C", Cat::Temperature, false, 1.0, false),
    ("cel", Cat::Temperature, false, 1.0, false),
    ("F", Cat::Temperature, false, 1.0, false),
    ("fah", Cat::Temperature, false, 1.0, false),
    ("Rank", Cat::Temperature, false, 1.0, false),
    ("Reau", Cat::Temperature, false, 1.0, false),
];

/// Metric prefixes as decimal exponents.
static METRIC: &[(&str, i32)] = &[
    ("Y", 24),
    ("Z", 21),
    ("E", 18),
    ("P", 15),
    ("T", 12),
    ("G", 9),
    ("M", 6),
    ("k", 3),
    ("h", 2),
    ("da", 1),
    ("e", 1),
    ("d", -1),
    ("c", -2),
    ("m", -3),
    ("u", -6),
    ("n", -9),
    ("p", -12),
    ("f", -15),
    ("a", -18),
    ("z", -21),
    ("y", -24),
];
/// Binary prefixes (bit and byte only) as powers of two.
static BINARY: &[(&str, i32)] = &[
    ("ki", 10),
    ("Mi", 20),
    ("Gi", 30),
    ("Ti", 40),
    ("Pi", 50),
    ("Ei", 60),
    ("Zi", 70),
    ("Yi", 80),
];

/// A resolved unit: category, how to apply the constant, the constant (prefix included) and, for temperatures,
/// the scale of the prefix.
#[derive(Debug, Clone, Copy)]
struct Unit {
    cat: Cat,
    divide: bool,
    constant: f64,
    name: &'static str,
}

fn find(name: &str) -> Option<&'static (&'static str, Cat, bool, f64, bool)> {
    UNITS.iter().find(|u| u.0 == name)
}

/// Every reading of a unit name: the exact name first, then prefix plus unit (`pc` is parsec or pico-calorie).
fn readings(name: &str) -> Vec<Unit> {
    let mut out = Vec::new();
    if let Some(u) = find(name) {
        out.push(Unit {
            cat: u.1,
            divide: u.2,
            constant: u.3,
            name: u.0,
        });
    }
    for (prefix, exp) in METRIC.iter().chain(BINARY) {
        let Some(rest) = name.strip_prefix(prefix) else {
            continue;
        };
        let Some(u) = find(rest) else { continue };
        let binary = BINARY.iter().any(|b| b.0 == *prefix);
        if !u.4 || (binary && !matches!(rest, "bit" | "byte")) {
            continue;
        }
        let power = if rest.ends_with('2') {
            2
        } else if rest.ends_with('3') {
            3
        } else {
            1
        };
        // Exact decimal power of the prefix, as Excel (the correctly rounded 10^(exp*power)).
        let scale = if binary {
            2f64.powi(exp * power)
        } else {
            format!("1e{}", exp * power).parse().unwrap_or(f64::NAN)
        };
        out.push(Unit {
            cat: u.1,
            divide: false,
            constant: if u.2 { scale / u.3 } else { scale * u.3 },
            name: u.0,
        });
    }
    out
}

/// Temperatures go through Celsius; the same unit (or its alias) is returned unchanged, as Excel does.
fn canonical(name: &str) -> &'static str {
    match name {
        "C" | "cel" => "C",
        "F" | "fah" => "F",
        "K" | "kel" => "K",
        "Rank" => "Rank",
        _ => "Reau",
    }
}

fn to_celsius(name: &str, x: f64) -> f64 {
    match canonical(name) {
        "K" => x - 273.15,
        "F" => (x - 32.0) * 5.0 / 9.0,
        "Rank" => x * 5.0 / 9.0 - 273.15,
        "Reau" => x * 1.25,
        _ => x,
    }
}

fn from_celsius(name: &str, c: f64) -> f64 {
    match canonical(name) {
        "K" => c + 273.15,
        "F" => c * 9.0 / 5.0 + 32.0,
        "Rank" => (c + 273.15) * 9.0 / 5.0,
        "Reau" => c * 0.8,
        _ => c,
    }
}

pub(super) fn convert(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 3, 3)?;
    // An omitted number is #N/A; a boolean anywhere is #VALUE! (the unit names are text, the number is not coerced).
    if matches!(args[0], Arg::Missing) {
        return Err(ErrorKind::NA);
    }
    for arg in args {
        if matches!(ctx.scalar(arg), Value::Bool(_)) {
            return Err(ErrorKind::Value);
        }
    }
    let x = num_arg(ctx, &args[0])?;
    let from = text_arg(ctx, &args[1])?;
    let to = text_arg(ctx, &args[2])?;
    let (a, b) = {
        let (fa, fb) = (readings(&from), readings(&to));
        let pair = fa
            .iter()
            .find_map(|a| fb.iter().find(|b| b.cat == a.cat).map(|b| (*a, *b)));
        pair.ok_or(ErrorKind::NA)?
    };
    let result = if from == to {
        // The same unit name gives the number back untouched, whatever the constants do to the last digit.
        x
    } else if a.cat == Cat::Temperature {
        // Prefixes on kelvin are ignored here; Excel applies one only on some paths (`CONVERT(0,"C","kK")` is 0.27315
        // but `CONVERT(37,"kK","Rank")` is 37 * 1.8), which is not modelled (listed in known-differences.toml).
        if canonical(a.name) == canonical(b.name) {
            x
        } else if a.name == "Rank" && b.name == "K" {
            x * 5.0 / 9.0
        } else {
            from_celsius(b.name, to_celsius(a.name, x))
        }
    } else {
        let y = if a.divide {
            x / a.constant
        } else {
            x * a.constant
        };
        if b.divide {
            y * b.constant
        } else {
            y / b.constant
        }
    };
    if result.is_finite() {
        Ok(Value::Number(result))
    } else {
        Err(ErrorKind::Num)
    }
}
