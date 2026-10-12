//! Calculation options: the workbook's `calcPr`.

use crate::XlsxError;
use crate::workbook::Workbook;
use crate::xml::{Attr, RawXml};

/// Formulas > Calculation Options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CalcMode {
    #[default]
    Automatic,
    /// `calcMode="autoNoTable"`: automatic except data tables.
    AutomaticExceptTables,
    Manual,
}

/// The `calcPr` settings Excel's Calculation Options and Excel Options > Formulas set.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CalcSettings {
    pub mode: CalcMode,
    /// `fullCalcOnLoad`: recalculate everything when the file opens.
    pub full_calc_on_load: bool,
    /// `calcOnSave` (on by default): recalculate before saving in manual mode.
    pub calc_on_save: bool,
    /// Enable iterative calculation.
    pub iterate: bool,
    pub iterate_count: u32,
    pub iterate_delta: f64,
    /// `fullPrecision` (on by default); off is "Set precision as displayed".
    pub full_precision: bool,
}

impl Default for CalcSettings {
    fn default() -> Self {
        Self {
            mode: CalcMode::Automatic,
            full_calc_on_load: false,
            calc_on_save: true,
            iterate: false,
            iterate_count: 100,
            iterate_delta: 0.001,
            full_precision: true,
        }
    }
}

fn truthy(v: &str) -> bool {
    matches!(v.trim(), "1" | "true")
}

impl Workbook {
    /// The calculation settings of the workbook.
    pub fn calc_settings(&self) -> CalcSettings {
        let mut s = CalcSettings::default();
        let Some(calc) = self
            .root
            .children
            .iter()
            .filter_map(RawXml::as_element)
            .find(|e| e.head.is_main("calcPr"))
        else {
            return s;
        };
        let h = &calc.head;
        s.mode = match h.attr("calcMode") {
            Some("manual") => CalcMode::Manual,
            Some("autoNoTable") => CalcMode::AutomaticExceptTables,
            _ => CalcMode::Automatic,
        };
        s.full_calc_on_load = h.attr("fullCalcOnLoad").is_some_and(truthy);
        s.calc_on_save = h.attr("calcOnSave").is_none_or(truthy);
        s.iterate = h.attr("iterate").is_some_and(truthy);
        if let Some(n) = h.attr("iterateCount").and_then(|v| v.trim().parse().ok()) {
            s.iterate_count = n;
        }
        if let Some(d) = h.attr("iterateDelta").and_then(|v| v.trim().parse().ok()) {
            s.iterate_delta = d;
        }
        s.full_precision = h.attr("fullPrecision").is_none_or(truthy);
        s
    }

    /// Writes the settings into `calcPr`, leaving attributes it does not model (`calcId`, `refMode`,
    /// `concurrentCalc` ...) as they are and dropping the ones that equal Excel's default.
    ///
    /// # Errors
    /// The iteration count is outside 1 to 32767 or the delta is not a positive number.
    pub fn set_calc_settings(&mut self, s: &CalcSettings) -> Result<(), XlsxError> {
        if s.iterate && !(1..=32_767).contains(&s.iterate_count) {
            return Err(XlsxError::UnsupportedEdit(
                "the iteration count must be between 1 and 32767".to_owned(),
            ));
        }
        if s.iterate && !(s.iterate_delta.is_finite() && s.iterate_delta > 0.0) {
            return Err(XlsxError::UnsupportedEdit(
                "the maximum change must be a positive number".to_owned(),
            ));
        }
        let mut calc = self
            .root
            .children
            .iter()
            .filter_map(RawXml::as_element)
            .find(|e| e.head.is_main("calcPr"))
            .cloned()
            .unwrap_or_else(|| {
                let mut c = self.root.sibling("calcPr");
                c.head.attrs.push(Attr::plain("calcId", "191029"));
                c
            });
        let h = &mut calc.head;
        let mut put = |name: &str, value: Option<String>| match value {
            Some(v) => h.set_attr(name, &v),
            None => h.remove_attr(name),
        };
        put(
            "calcMode",
            match s.mode {
                CalcMode::Automatic => None,
                CalcMode::AutomaticExceptTables => Some("autoNoTable".to_owned()),
                CalcMode::Manual => Some("manual".to_owned()),
            },
        );
        put(
            "fullCalcOnLoad",
            s.full_calc_on_load.then(|| "1".to_owned()),
        );
        put("calcOnSave", (!s.calc_on_save).then(|| "0".to_owned()));
        put("iterate", s.iterate.then(|| "1".to_owned()));
        put(
            "iterateCount",
            (s.iterate && s.iterate_count != 100).then(|| s.iterate_count.to_string()),
        );
        put(
            "iterateDelta",
            (s.iterate && (s.iterate_delta - 0.001).abs() > f64::EPSILON)
                .then(|| format!("{}", s.iterate_delta)),
        );
        put("fullPrecision", (!s.full_precision).then(|| "0".to_owned()));
        self.put_workbook_child("calcPr", Some(calc));
        Ok(())
    }

    /// Switches between automatic and manual calculation (Formulas > Calculation Options).
    ///
    /// # Errors
    /// See [`Workbook::set_calc_settings`].
    pub fn set_calc_mode(&mut self, mode: CalcMode) -> Result<(), XlsxError> {
        let mut s = self.calc_settings();
        s.mode = mode;
        self.set_calc_settings(&s)
    }

    /// Sets or clears `fullCalcOnLoad`.
    ///
    /// # Errors
    /// See [`Workbook::set_calc_settings`].
    pub fn request_full_calc_on_load(&mut self, on: bool) -> Result<(), XlsxError> {
        let mut s = self.calc_settings();
        s.full_calc_on_load = on;
        self.set_calc_settings(&s)
    }
}
