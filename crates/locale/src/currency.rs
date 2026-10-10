//! Currency symbol and patterns per region.

use icu_experimental::dimension::currency::CurrencyType;
use icu_experimental::dimension::currency::formatter::{
    CurrencyFormatter, CurrencyFormatterPreferences,
};
use icu_locale::LocaleExpander;

use crate::numbers::Digits;
use crate::{Locale, LocaleError};

/// Currency conventions for a locale's region.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CurrencyFormat {
    /// ISO 4217 code of the region's currency (`EUR`, `USD`).
    pub code: String,
    /// Symbol as the locale writes it (`€`, `$`, `kr.`).
    pub symbol: String,
    /// Positive pattern: `¤` marks the symbol and `#` the number, for example `¤#` or `# ¤`.
    pub positive_pattern: String,
    /// Negative pattern in the same notation, for example `-¤#`, `(¤#)` or `-# ¤`. The minus
    /// sign is written as ASCII `-` whatever the locale's minus glyph is.
    pub negative_pattern: String,
    /// Fraction digits of the locale's own rendering of 1 in this currency (2, 0 for JPY).
    pub decimals: u8,
}

/// ISO 4217 code for a region (ISO 3166-1 alpha-2), following CLDR's current-currency data.
fn region_currency(region: &str) -> Option<&'static str> {
    Some(match region {
        "AT" | "BE" | "BG" | "CY" | "DE" | "EE" | "ES" | "FI" | "FR" | "GR" | "HR" | "IE"
        | "IT" | "LT" | "LU" | "LV" | "MC" | "ME" | "MT" | "NL" | "PT" | "SI" | "SK" | "SM"
        | "VA" | "AD" | "AX" | "BL" | "GF" | "GP" | "MF" | "MQ" | "PM" | "RE" | "YT" | "EU"
        | "XK" => "EUR",
        "US" | "AS" | "EC" | "FM" | "GU" | "MH" | "MP" | "PR" | "PW" | "SV" | "TL" | "UM"
        | "VI" | "BQ" | "IO" | "TC" | "VG" => "USD",
        "GB" | "GG" | "IM" | "JE" => "GBP",
        "AE" => "AED",
        "AF" => "AFN",
        "AL" => "ALL",
        "AM" => "AMD",
        "AO" => "AOA",
        "AR" => "ARS",
        "AU" | "CC" | "CX" | "KI" | "NF" | "NR" | "TV" => "AUD",
        "AZ" => "AZN",
        "BA" => "BAM",
        "BD" => "BDT",
        "BF" | "BJ" | "CI" | "GW" | "ML" | "NE" | "SN" | "TG" => "XOF",
        "BH" => "BHD",
        "BN" => "BND",
        "BO" => "BOB",
        "BR" => "BRL",
        "BS" => "BSD",
        "BT" => "BTN",
        "BW" => "BWP",
        "BY" => "BYN",
        "BZ" => "BZD",
        "CA" => "CAD",
        "CD" => "CDF",
        "CF" | "CG" | "CM" | "GA" | "GQ" | "TD" => "XAF",
        "CH" | "LI" => "CHF",
        "CL" => "CLP",
        "CN" => "CNY",
        "CO" => "COP",
        "CR" => "CRC",
        "CU" => "CUP",
        "CV" => "CVE",
        "CZ" => "CZK",
        "DJ" => "DJF",
        "DK" | "FO" | "GL" => "DKK",
        "DO" => "DOP",
        "DZ" => "DZD",
        "EG" => "EGP",
        "ER" => "ERN",
        "ET" => "ETB",
        "FJ" => "FJD",
        "GE" => "GEL",
        "GH" => "GHS",
        "GM" => "GMD",
        "GN" => "GNF",
        "GT" => "GTQ",
        "GY" => "GYD",
        "HK" => "HKD",
        "HN" => "HNL",
        "HT" => "HTG",
        "HU" => "HUF",
        "ID" => "IDR",
        "IL" | "PS" => "ILS",
        "IN" => "INR",
        "IQ" => "IQD",
        "IR" => "IRR",
        "IS" => "ISK",
        "JM" => "JMD",
        "JO" => "JOD",
        "JP" => "JPY",
        "KE" => "KES",
        "KG" => "KGS",
        "KH" => "KHR",
        "KM" => "KMF",
        "KP" => "KPW",
        "KR" => "KRW",
        "KW" => "KWD",
        "KY" => "KYD",
        "KZ" => "KZT",
        "LA" => "LAK",
        "LB" => "LBP",
        "LK" => "LKR",
        "LR" => "LRD",
        "LS" => "LSL",
        "LY" => "LYD",
        "MA" | "EH" => "MAD",
        "MD" => "MDL",
        "MG" => "MGA",
        "MK" => "MKD",
        "MM" => "MMK",
        "MN" => "MNT",
        "MO" => "MOP",
        "MR" => "MRU",
        "MU" => "MUR",
        "MV" => "MVR",
        "MW" => "MWK",
        "MX" => "MXN",
        "MY" => "MYR",
        "MZ" => "MZN",
        "NA" => "NAD",
        "NG" => "NGN",
        "NI" => "NIO",
        "NO" | "SJ" | "BV" => "NOK",
        "NP" => "NPR",
        "NZ" | "CK" | "NU" | "PN" | "TK" => "NZD",
        "OM" => "OMR",
        "PA" => "PAB",
        "PE" => "PEN",
        "PG" => "PGK",
        "PH" => "PHP",
        "PK" => "PKR",
        "PL" => "PLN",
        "PY" => "PYG",
        "QA" => "QAR",
        "RO" => "RON",
        "RS" => "RSD",
        "RU" => "RUB",
        "RW" => "RWF",
        "SA" => "SAR",
        "SB" => "SBD",
        "SC" => "SCR",
        "SD" => "SDG",
        "SE" => "SEK",
        "SG" => "SGD",
        "SL" => "SLE",
        "SO" => "SOS",
        "SR" => "SRD",
        "SS" => "SSP",
        "ST" => "STN",
        "SY" => "SYP",
        "SZ" => "SZL",
        "TH" => "THB",
        "TJ" => "TJS",
        "TM" => "TMT",
        "TN" => "TND",
        "TO" => "TOP",
        "TR" => "TRY",
        "TT" => "TTD",
        "TW" => "TWD",
        "TZ" => "TZS",
        "UA" => "UAH",
        "UG" => "UGX",
        "UY" => "UYU",
        "UZ" => "UZS",
        "VE" => "VES",
        "VN" => "VND",
        "VU" => "VUV",
        "WS" => "WST",
        "YE" => "YER",
        "ZA" => "ZAR",
        "ZM" => "ZMW",
        "ZW" => "ZWL",
        _ => return None,
    })
}

/// Region used to pick a currency: the tag's region, else the likely region of the language.
fn currency_region(locale: &Locale) -> String {
    if let Some(region) = locale.icu().id.region {
        return region.as_str().to_owned();
    }
    let mut id = locale.icu().id.clone();
    LocaleExpander::new_common().maximize(&mut id);
    id.region
        .map_or_else(|| "US".to_owned(), |r| r.as_str().to_owned())
}

impl Locale {
    /// Currency conventions for this locale's region (`en-US` gives `USD`, `¤#`).
    pub fn currency_format(&self) -> Result<CurrencyFormat, LocaleError> {
        let region = currency_region(self);
        let code = region_currency(&region).unwrap_or("USD");
        let currency: CurrencyType = code
            .parse()
            .map_err(|_| LocaleError::Data(format!("currency code {code}")))?;
        let prefs = CurrencyFormatterPreferences::from(&self.icu);
        let fmt = CurrencyFormatter::try_new_symbol(prefs, currency, Default::default())
            .map_err(|e| LocaleError::Data(format!("currency: {e}")))?;
        let d = Digits::new(self, false)?;
        let positive = fmt
            .format_fixed_decimal(&crate::numbers::parse_decimal("1")?)
            .to_string();
        let negative = fmt
            .format_fixed_decimal(&crate::numbers::parse_decimal("-1")?)
            .to_string();
        let symbol = d.symbol_of(&positive);
        Ok(CurrencyFormat {
            code: code.to_owned(),
            positive_pattern: d.pattern_of(&positive, &symbol, "\u{a4}"),
            negative_pattern: d.pattern_of(&negative, &symbol, "\u{a4}"),
            symbol,
            decimals: d.fraction_digits(&positive).min(9) as u8,
        })
    }
}
