use core::cmp::Ordering;

use super::*;

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn lcid_table_is_complete_and_parses() -> TestResult {
    let all: Vec<_> = Locale::all_lcids().collect();
    let unique: std::collections::BTreeSet<u32> = all.iter().map(|(l, _)| *l).collect();
    assert_eq!(unique.len(), 418, "assigned LCIDs in [MS-LCID]");
    for (lcid, tag) in all {
        let locale = Locale::from_lcid(lcid)?;
        assert_eq!(locale.lcid(), Some(lcid));
        // The tag round-trips to some LCID with the same tag.
        let again = Locale::from_bcp47(tag)?;
        assert!(again.lcid().is_some(), "{tag} has no LCID");
    }
    Ok(())
}

#[test]
fn tag_and_lcid_agree() -> TestResult {
    assert_eq!(Locale::from_bcp47("de-DE")?.lcid(), Some(0x0407));
    assert_eq!(Locale::from_lcid(0x0807)?.bcp47(), "de-CH");
    assert_eq!(
        Locale::from_lcid(0x10407)?.bcp47(),
        "de-DE",
        "sort-order bits are ignored"
    );
    assert_eq!(
        Locale::from_bcp47("en-US-u-ca-hebrew")?.lcid(),
        Some(0x0409)
    );
    assert_eq!(Locale::from_lcid(0x0467)?.bcp47(), "ff-Latn-NG");
    assert_eq!(Locale::from_bcp47("tlh").map(|l| l.lcid()), Ok(None));
    assert!(matches!(
        Locale::from_lcid(0x7777),
        Err(LocaleError::UnknownLcid(0x7777))
    ));
    assert!(matches!(
        Locale::from_bcp47("not a tag"),
        Err(LocaleError::InvalidTag(_))
    ));
    assert_eq!(Locale::en_us(), Locale::from_lcid(0x0409)?);
    Ok(())
}

#[test]
fn every_locale_builds_all_data() -> TestResult {
    for (lcid, _) in Locale::all_lcids() {
        let locale = Locale::from_lcid(lcid)?;
        let n = locale.number_format()?;
        assert!(
            !n.decimal_separator.is_empty(),
            "{lcid:#x} decimal separator"
        );
        assert!(!n.currency.symbol.is_empty(), "{lcid:#x} currency symbol");
        assert!(n.currency.positive_pattern.contains('#'));
        locale.date_time_patterns()?;
        locale.calendar_names()?;
        locale.first_day_of_week()?;
        assert!(
            locale
                .supported_calendars()
                .contains(&locale.default_calendar())
        );
        locale.collator()?;
    }
    Ok(())
}

#[test]
fn number_conventions() -> TestResult {
    let us = Locale::from_bcp47("en-US")?.number_format()?;
    assert_eq!(
        (us.decimal_separator.as_str(), us.group_separator.as_str()),
        (".", ",")
    );
    assert_eq!((us.primary_grouping, us.secondary_grouping), (3, 3));
    assert_eq!(us.list_separator, ",");
    assert_eq!(us.percent_pattern, "#%");
    assert_eq!(us.currency.code, "USD");
    assert_eq!(
        (
            us.currency.symbol.as_str(),
            us.currency.positive_pattern.as_str()
        ),
        ("$", "\u{a4}#")
    );
    assert_eq!(us.currency.negative_pattern, "-\u{a4}#");
    assert_eq!(us.currency.decimals, 2);

    let de = Locale::from_bcp47("de-DE")?.number_format()?;
    assert_eq!(
        (de.decimal_separator.as_str(), de.group_separator.as_str()),
        (",", ".")
    );
    assert_eq!(de.list_separator, ";");
    assert_eq!(de.currency.positive_pattern, "#\u{a0}\u{a4}");

    let india = Locale::from_bcp47("en-IN")?.number_format()?;
    assert_eq!((india.primary_grouping, india.secondary_grouping), (3, 2));

    let sv = Locale::from_bcp47("sv-SE")?.number_format()?;
    assert_eq!(
        sv.minus_sign, "-",
        "U+2212 is normalised to the ASCII minus Excel writes"
    );

    let ar = Locale::from_bcp47("ar-EG")?.number_format()?;
    assert_eq!(ar.digits[1], '\u{661}');
    let jp = Locale::from_bcp47("ja-JP")?.number_format()?;
    assert_eq!(jp.currency.decimals, 0);
    Ok(())
}

#[test]
fn names_use_office_overrides() -> TestResult {
    let de = Locale::from_bcp47("de-DE")?.calendar_names()?;
    assert_eq!(de.month_abbr[2], "Mrz", "measured Excel abbreviation");
    assert_eq!(de.day_full[0], "Sonntag");
    let cldr = Locale::from_bcp47("de-DE")?.calendar_names_cldr()?;
    assert_eq!(cldr.month_abbr[2], "M\u{e4}r");
    let ru = Locale::from_bcp47("ru-RU")?.calendar_names()?;
    assert_eq!(ru.month_full[2], "\u{41c}\u{430}\u{440}\u{442}");
    assert_eq!(
        ru.month_full_genitive[2],
        "\u{43c}\u{430}\u{440}\u{442}\u{430}"
    );
    Ok(())
}

#[test]
fn date_patterns_and_week() -> TestResult {
    let us = Locale::from_bcp47("en-US")?;
    assert_eq!(us.date_time_patterns()?.long_date, "MMMM d, y");
    assert_eq!(us.first_day_of_week()?, Weekday::Sunday);
    assert_eq!(
        Locale::from_bcp47("de-DE")?.first_day_of_week()?,
        Weekday::Monday
    );
    assert_eq!(
        Locale::from_bcp47("de-DE")?.date_time_patterns()?.time,
        "HH:mm:ss"
    );
    Ok(())
}

#[test]
fn calendars_and_direction() -> TestResult {
    assert_eq!(
        Locale::from_bcp47("th-TH")?.default_calendar(),
        CalendarKind::ThaiBuddhist
    );
    assert_eq!(
        Locale::from_bcp47("en-US")?.default_calendar(),
        CalendarKind::Gregorian
    );
    assert!(
        Locale::from_bcp47("ja-JP")?
            .supported_calendars()
            .contains(&CalendarKind::JapaneseEra)
    );
    assert!(
        Locale::from_bcp47("ar-SA")?
            .supported_calendars()
            .contains(&CalendarKind::Hijri)
    );
    assert_eq!(
        Locale::from_bcp47("ar-SA")?.text_direction(),
        TextDirection::RightToLeft
    );
    assert_eq!(
        Locale::from_bcp47("he-IL")?.text_direction(),
        TextDirection::RightToLeft
    );
    assert_eq!(
        Locale::from_bcp47("en-US")?.text_direction(),
        TextDirection::LeftToRight
    );
    Ok(())
}

#[test]
fn case_mapping_follows_locale() -> TestResult {
    let tr = Locale::from_bcp47("tr-TR")?;
    assert_eq!(tr.to_uppercase("istanbul"), "\u{130}STANBUL");
    assert_eq!(tr.to_lowercase("I"), "\u{131}");
    let en = Locale::from_bcp47("en-US")?;
    assert_eq!(en.to_uppercase("istanbul"), "ISTANBUL");
    assert_eq!(en.to_uppercase("stra\u{df}e"), "STRASSE");
    Ok(())
}

#[test]
fn collation_follows_locale() -> TestResult {
    let de = Locale::from_bcp47("de-DE")?.collator()?;
    let sv = Locale::from_bcp47("sv-SE")?.collator()?;
    assert_eq!(de.compare("\u{e4}", "z"), Ordering::Less);
    assert_eq!(
        sv.compare("\u{e4}", "z"),
        Ordering::Greater,
        "Swedish sorts a-umlaut after z"
    );
    let loose = Locale::from_bcp47("en-US")?.collator_with(CollationStrength::Secondary)?;
    assert_eq!(loose.compare("abc", "ABC"), Ordering::Equal);
    Ok(())
}
