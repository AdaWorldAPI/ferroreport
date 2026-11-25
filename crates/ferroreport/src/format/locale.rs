//! Locale data needed by the number and date formatters (a small CLDR subset).

/// Symbols and names for one locale.
#[derive(Debug, Clone)]
pub struct Locale {
    pub tag: &'static str,
    pub decimal_separator: char,
    pub grouping_separator: char,
    pub currency_symbol: &'static str,
    pub months: [&'static str; 12],
    pub short_months: [&'static str; 12],
    /// Sunday first, like `java.util.Calendar`.
    pub weekdays: [&'static str; 7],
    pub short_weekdays: [&'static str; 7],
    pub am_pm: [&'static str; 2],
}

const EN_MONTHS: [&str; 12] =
    ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const EN_SHORT_MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const EN_WEEKDAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const EN_SHORT_WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

const ES_MONTHS: [&str; 12] =
    ["enero", "febrero", "marzo", "abril", "mayo", "junio", "julio", "agosto", "septiembre", "octubre", "noviembre", "diciembre"];
const ES_SHORT_MONTHS: [&str; 12] = ["ene", "feb", "mar", "abr", "may", "jun", "jul", "ago", "sept", "oct", "nov", "dic"];
const ES_WEEKDAYS: [&str; 7] = ["domingo", "lunes", "martes", "miércoles", "jueves", "viernes", "sábado"];
const ES_SHORT_WEEKDAYS: [&str; 7] = ["dom", "lun", "mar", "mié", "jue", "vie", "sáb"];

const DE_MONTHS: [&str; 12] =
    ["Januar", "Februar", "März", "April", "Mai", "Juni", "Juli", "August", "September", "Oktober", "November", "Dezember"];
const DE_SHORT_MONTHS: [&str; 12] =
    ["Jan.", "Feb.", "März", "Apr.", "Mai", "Juni", "Juli", "Aug.", "Sept.", "Okt.", "Nov.", "Dez."];
const DE_WEEKDAYS: [&str; 7] = ["Sonntag", "Montag", "Dienstag", "Mittwoch", "Donnerstag", "Freitag", "Samstag"];
const DE_SHORT_WEEKDAYS: [&str; 7] = ["So.", "Mo.", "Di.", "Mi.", "Do.", "Fr.", "Sa."];

pub const EN_US: Locale = Locale {
    tag: "en_US",
    decimal_separator: '.',
    grouping_separator: ',',
    currency_symbol: "$",
    months: EN_MONTHS,
    short_months: EN_SHORT_MONTHS,
    weekdays: EN_WEEKDAYS,
    short_weekdays: EN_SHORT_WEEKDAYS,
    am_pm: ["AM", "PM"],
};

/// Spanish with the Latin-American number symbols used in DO, MX and US.
pub const ES_DO: Locale = Locale {
    tag: "es_DO",
    decimal_separator: '.',
    grouping_separator: ',',
    currency_symbol: "RD$",
    months: ES_MONTHS,
    short_months: ES_SHORT_MONTHS,
    weekdays: ES_WEEKDAYS,
    short_weekdays: ES_SHORT_WEEKDAYS,
    am_pm: ["a.\u{202F}m.", "p.\u{202F}m."], // Java 21 (CLDR) uses a narrow no-break space
};

pub const ES_MX: Locale = Locale { tag: "es_MX", currency_symbol: "$", ..ES_DO };

/// Spanish (Spain): comma decimal separator, dot grouping.
pub const ES_ES: Locale =
    Locale { tag: "es_ES", decimal_separator: ',', grouping_separator: '.', currency_symbol: "€", ..ES_DO };

/// German: comma decimal separator, dot grouping (values from Java 21 `DecimalFormatSymbols` / `DateFormatSymbols`).
pub const DE: Locale = Locale {
    tag: "de",
    decimal_separator: ',',
    grouping_separator: '.',
    currency_symbol: "€",
    months: DE_MONTHS,
    short_months: DE_SHORT_MONTHS,
    weekdays: DE_WEEKDAYS,
    short_weekdays: DE_SHORT_WEEKDAYS,
    am_pm: ["AM", "PM"],
};

/// German (Germany); same symbols as `DE` (values from Java 21 `DecimalFormatSymbols` / `DateFormatSymbols`).
pub const DE_DE: Locale = Locale { tag: "de_DE", ..DE };

/// German (Austria): no-break space grouping, "Jänner" (values from Java 21 `DecimalFormatSymbols` / `DateFormatSymbols`).
pub const DE_AT: Locale = Locale {
    tag: "de_AT",
    grouping_separator: '\u{A0}',
    months: [
        "Jänner",
        "Februar",
        "März",
        "April",
        "Mai",
        "Juni",
        "Juli",
        "August",
        "September",
        "Oktober",
        "November",
        "Dezember",
    ],
    short_months: ["Jän.", "Feb.", "März", "Apr.", "Mai", "Juni", "Juli", "Aug.", "Sep.", "Okt.", "Nov.", "Dez."],
    ..DE
};

/// German (Switzerland): dot decimal, apostrophe grouping, CHF (values from Java 21 `DecimalFormatSymbols` / `DateFormatSymbols`).
pub const DE_CH: Locale =
    Locale { tag: "de_CH", decimal_separator: '.', grouping_separator: '\u{2019}', currency_symbol: "CHF", ..DE };

impl Locale {
    /// Resolves a Java-style locale tag (`en_US`, `es-DO`, `es`). Unknown tags fall back to `en_US`.
    pub fn from_tag(tag: &str) -> &'static Locale {
        let norm = tag.replace('-', "_");
        match norm.as_str() {
            "es_DO" => &ES_DO,
            "es_MX" | "es_US" | "es_419" => &ES_MX,
            "es" | "es_ES" => &ES_ES,
            "de_DE" => &DE_DE,
            "de_AT" => &DE_AT,
            "de_CH" | "de_LI" => &DE_CH,
            "de" => &DE,
            _ if norm.starts_with("de_") => &DE_DE,
            _ if norm.starts_with("es_") => &ES_DO,
            _ => &EN_US,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn german_tags_resolve() {
        assert_eq!(Locale::from_tag("de_DE").tag, "de_DE");
        assert_eq!(Locale::from_tag("de-AT").tag, "de_AT");
        assert_eq!(Locale::from_tag("de_LI").tag, "de_CH");
        assert_eq!(Locale::from_tag("de_LU").tag, "de_DE");
        assert_eq!(Locale::from_tag("de").tag, "de");
        assert_eq!(Locale::from_tag("fr_FR").tag, "en_US");
    }
}
