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

impl Locale {
    /// Resolves a Java-style locale tag (`en_US`, `es-DO`, `es`). Unknown tags fall back to `en_US`.
    pub fn from_tag(tag: &str) -> &'static Locale {
        let norm = tag.replace('-', "_");
        match norm.as_str() {
            "es_DO" => &ES_DO,
            "es_MX" | "es_US" | "es_419" => &ES_MX,
            "es" | "es_ES" => &ES_ES,
            _ if norm.starts_with("es_") => &ES_DO,
            _ => &EN_US,
        }
    }
}
