//! Request-scoped locale state, shared between the VM and the stdlib.
//!
//! GH #304: `getLocale()` returned a hardcoded `en_US`, `setLocale()` computed a
//! code and dropped it, and the cfconfig `runtime.locale` key parsed but had no
//! consumer — so every `ls*` formatter behaved as US English no matter what the
//! application asked for.
//!
//! The `ls*` functions live in `cfml-stdlib` as pure `Vec<CfmlValue> -> CfmlResult`
//! builtins with no handle on the VM, while the locale itself is VM request state
//! (`Vm::locale`). This module is the seam: the VM publishes the active locale here
//! whenever it changes (`setLocale()`, cfconfig apply, request-seed restore) and the
//! formatters read it as their default. Thread-local, matching how the engine already
//! scopes per-request state such as `REQUEST_MYSQL_CONNS` — serve mode handles one
//! request per worker thread, so a locale set by one request cannot leak into
//! another's formatting.

use std::cell::RefCell;

thread_local! {
    /// Active locale code (e.g. `en_GB`) for this thread's request. Empty means
    /// "not set" — callers fall back to [`DEFAULT_LOCALE`].
    static CURRENT_LOCALE: RefCell<String> = const { RefCell::new(String::new()) };
}

/// The locale assumed when neither cfconfig nor `setLocale()` has named one.
/// Matches Lucee's own default.
pub const DEFAULT_LOCALE: &str = "en_US";

/// Publish the active locale for this thread. `code` is a canonical
/// underscore-form code such as `en_GB`; an empty string resets to the default.
pub fn set_current_locale(code: &str) {
    CURRENT_LOCALE.with(|c| *c.borrow_mut() = code.to_string());
}

/// The active locale code, or [`DEFAULT_LOCALE`] when none has been set.
pub fn current_locale() -> String {
    CURRENT_LOCALE.with(|c| {
        let v = c.borrow();
        if v.is_empty() {
            DEFAULT_LOCALE.to_string()
        } else {
            v.clone()
        }
    })
}

/// Normalise the many spellings CFML accepts for a locale — Java-style
/// (`en_GB`), BCP 47 (`en-GB`), and the ColdFusion "friendly" names
/// (`English (UK)`) — to a canonical `ll_CC` code.
///
/// Returns `None` for input that names no locale we can resolve, so callers can
/// raise an error rather than silently formatting as US English.
pub fn canonical_locale(name: &str) -> Option<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_lowercase();
    let friendly = match lower.as_str() {
        "english (us)" | "english (united states)" => Some("en_US"),
        "english (uk)" | "english (united kingdom)" => Some("en_GB"),
        "english (australian)" => Some("en_AU"),
        "english (canadian)" => Some("en_CA"),
        "english (new zealand)" => Some("en_NZ"),
        "german (standard)" | "german" => Some("de_DE"),
        "german (austrian)" => Some("de_AT"),
        "german (swiss)" => Some("de_CH"),
        "french (standard)" | "french" => Some("fr_FR"),
        "french (canadian)" => Some("fr_CA"),
        "french (belgian)" => Some("fr_BE"),
        "french (swiss)" => Some("fr_CH"),
        "spanish (standard)" | "spanish" => Some("es_ES"),
        "spanish (mexican)" => Some("es_MX"),
        "italian (standard)" | "italian" => Some("it_IT"),
        "italian (swiss)" => Some("it_CH"),
        "portuguese (standard)" | "portuguese" => Some("pt_PT"),
        "portuguese (brazilian)" => Some("pt_BR"),
        "dutch (standard)" | "dutch" => Some("nl_NL"),
        "dutch (belgian)" => Some("nl_BE"),
        "swedish" => Some("sv_SE"),
        "norwegian (bokmal)" | "norwegian" => Some("nb_NO"),
        "danish" => Some("da_DK"),
        "finnish" => Some("fi_FI"),
        "polish" => Some("pl_PL"),
        "russian" => Some("ru_RU"),
        "turkish" => Some("tr_TR"),
        "japanese" => Some("ja_JP"),
        "korean" => Some("ko_KR"),
        "chinese (china)" | "chinese" => Some("zh_CN"),
        "chinese (taiwan)" => Some("zh_TW"),
        _ => None,
    };
    if let Some(code) = friendly {
        return Some(code.to_string());
    }

    // Structural form: `ll`, `ll_CC` or `ll-CC`. Language lowercase, region upper.
    let parts: Vec<&str> = trimmed.split(['_', '-']).filter(|p| !p.is_empty()).collect();
    match parts.as_slice() {
        [lang] if lang.len() == 2 && lang.chars().all(|c| c.is_ascii_alphabetic()) => {
            // A bare language picks its conventional default region.
            Some(default_region_for(&lang.to_lowercase()))
        }
        [lang, region]
            if lang.len() == 2
                && region.len() == 2
                && lang.chars().all(|c| c.is_ascii_alphabetic())
                && region.chars().all(|c| c.is_ascii_alphabetic()) =>
        {
            Some(format!("{}_{}", lang.to_lowercase(), region.to_uppercase()))
        }
        _ => None,
    }
}

fn default_region_for(lang: &str) -> String {
    let region = match lang {
        "en" => "US",
        "de" => "DE",
        "fr" => "FR",
        "es" => "ES",
        "it" => "IT",
        "pt" => "PT",
        "nl" => "NL",
        "sv" => "SE",
        "nb" | "no" => "NO",
        "da" => "DK",
        "fi" => "FI",
        "pl" => "PL",
        "ru" => "RU",
        "tr" => "TR",
        "ja" => "JP",
        "ko" => "KR",
        "zh" => "CN",
        other => return other.to_uppercase(),
    };
    format!("{}_{}", lang, region)
}

/// The ColdFusion "friendly" display name for a canonical code — what
/// `getLocale()` returns. Unknown codes echo back lowercased, as Lucee does.
pub fn friendly_name(code: &str) -> String {
    match code {
        "en_US" => "english (us)",
        "en_GB" => "english (uk)",
        "en_AU" => "english (australian)",
        "en_CA" => "english (canadian)",
        "en_NZ" => "english (new zealand)",
        "de_DE" => "german (standard)",
        "de_AT" => "german (austrian)",
        "de_CH" => "german (swiss)",
        "fr_FR" => "french (standard)",
        "fr_CA" => "french (canadian)",
        "fr_BE" => "french (belgian)",
        "fr_CH" => "french (swiss)",
        "es_ES" => "spanish (standard)",
        "es_MX" => "spanish (mexican)",
        "it_IT" => "italian (standard)",
        "it_CH" => "italian (swiss)",
        "pt_PT" => "portuguese (standard)",
        "pt_BR" => "portuguese (brazilian)",
        "nl_NL" => "dutch (standard)",
        "nl_BE" => "dutch (belgian)",
        "sv_SE" => "swedish",
        "nb_NO" => "norwegian (bokmal)",
        "da_DK" => "danish",
        "fi_FI" => "finnish",
        "pl_PL" => "polish",
        "ru_RU" => "russian",
        "tr_TR" => "turkish",
        "ja_JP" => "japanese",
        "ko_KR" => "korean",
        "zh_CN" => "chinese (china)",
        "zh_TW" => "chinese (taiwan)",
        other => return other.to_lowercase(),
    }
    .to_string()
}

/// How a locale punctuates and lays out numbers and currency.
#[derive(Debug, Clone, Copy)]
pub struct LocaleNumberFormat {
    /// Separator between the integer and fractional parts.
    pub decimal: char,
    /// Thousands-grouping separator. `'\u{0}'` means "no grouping".
    pub grouping: char,
    /// ISO 4217 code used by `lsCurrencyFormat(n, "international")`.
    pub currency_code: &'static str,
    /// Symbol used by `lsCurrencyFormat(n, "local")`.
    pub currency_symbol: &'static str,
    /// True when the symbol trails the amount (`1.234,50 €`) rather than
    /// leading it (`£1,234.50`).
    pub symbol_after: bool,
    /// True when a space sits between the amount and the symbol.
    pub space_before_symbol: bool,
    /// Digits after the decimal point for currency amounts (0 for JPY/KRW).
    pub currency_decimals: usize,
}

impl LocaleNumberFormat {
    /// The `en_US` conventions — the behaviour every `ls*` function had
    /// hardcoded before GH #304.
    pub const US: LocaleNumberFormat = LocaleNumberFormat {
        decimal: '.',
        grouping: ',',
        currency_code: "USD",
        currency_symbol: "$",
        symbol_after: false,
        space_before_symbol: false,
        currency_decimals: 2,
    };
}

/// Number/currency conventions for a canonical locale code.
///
/// Deliberately a hand-maintained table rather than a full CLDR/ICU dependency:
/// it covers the locales RustCFML already names in [`friendly_name`], and an
/// unlisted locale falls back to its language's conventions (and finally to
/// `en_US`) rather than to silence. Extend the table when a locale is needed —
/// do NOT let a caller's locale argument be dropped.
pub fn number_format_for(code: &str) -> LocaleNumberFormat {
    // Currency is region-determined; punctuation is mostly language-determined.
    let (currency_code, currency_symbol, currency_decimals) = match code {
        "en_US" | "es_MX" => ("USD", "$", 2),
        "en_GB" => ("GBP", "£", 2),
        "en_AU" => ("AUD", "$", 2),
        "en_CA" | "fr_CA" => ("CAD", "$", 2),
        "en_NZ" => ("NZD", "$", 2),
        "de_DE" | "fr_FR" | "fr_BE" | "es_ES" | "it_IT" | "pt_PT" | "nl_NL" | "nl_BE"
        | "de_AT" | "fi_FI" => ("EUR", "€", 2),
        "de_CH" | "fr_CH" | "it_CH" => ("CHF", "CHF", 2),
        "pt_BR" => ("BRL", "R$", 2),
        "sv_SE" => ("SEK", "kr", 2),
        "nb_NO" => ("NOK", "kr", 2),
        "da_DK" => ("DKK", "kr", 2),
        "pl_PL" => ("PLN", "zł", 2),
        "ru_RU" => ("RUB", "₽", 2),
        "tr_TR" => ("TRY", "₺", 2),
        // Yen and Won have no minor unit.
        // Java/Lucee render JPY with the FULLWIDTH yen sign (U+FFE5) in ja_JP,
        // not the halfwidth ¥ (U+00A5) that zh_CN uses for CNY.
        "ja_JP" => ("JPY", "￥", 0),
        "ko_KR" => ("KRW", "₩", 0),
        "zh_CN" => ("CNY", "¥", 2),
        "zh_TW" => ("TWD", "NT$", 2),
        _ => ("USD", "$", 2),
    };

    let lang = code.split('_').next().unwrap_or("en");
    // Comma-decimal languages, and the space-grouping ones among them.
    let (decimal, grouping) = match (lang, code) {
        (_, "de_CH") | (_, "fr_CH") | (_, "it_CH") => ('.', '\''),
        ("fr", _) => (',', '\u{202f}'), // narrow no-break space
        ("de", _) | ("es", _) | ("it", _) | ("pt", _) | ("nl", _) | ("tr", _) => (',', '.'),
        ("sv", _) | ("nb", _) | ("fi", _) | ("pl", _) | ("ru", _) => (',', '\u{a0}'),
        ("da", _) => (',', '.'),
        _ => ('.', ','),
    };

    // Most comma-decimal European locales trail the symbol; anglophone and CJK
    // locales lead with it.
    let symbol_after = matches!(
        lang,
        "de" | "fr" | "es" | "it" | "pt" | "sv" | "nb" | "da" | "fi" | "pl" | "ru" | "tr"
    ) && !matches!(code, "pt_BR");
    let space_before_symbol = symbol_after || matches!(code, "pt_BR" | "de_CH" | "fr_CH" | "it_CH");

    LocaleNumberFormat {
        decimal,
        grouping,
        currency_code,
        currency_symbol,
        symbol_after,
        space_before_symbol,
        currency_decimals,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalises_every_accepted_spelling() {
        for spelling in ["en_GB", "en-GB", "English (UK)", "EN_gb"] {
            assert_eq!(canonical_locale(spelling).as_deref(), Some("en_GB"), "{spelling}");
        }
        assert_eq!(canonical_locale("de").as_deref(), Some("de_DE"));
        // Unresolvable input must be reported, never silently defaulted.
        assert_eq!(canonical_locale("not a locale"), None);
        assert_eq!(canonical_locale("   "), None);
    }

    #[test]
    fn friendly_name_round_trips() {
        for code in ["en_US", "en_GB", "de_DE", "ja_JP"] {
            let friendly = friendly_name(code);
            assert_eq!(canonical_locale(&friendly).as_deref(), Some(code), "{code}");
        }
    }

    #[test]
    fn number_formats_differ_by_locale() {
        let gb = number_format_for("en_GB");
        assert_eq!(gb.currency_symbol, "£");
        assert!(!gb.symbol_after);

        let de = number_format_for("de_DE");
        assert_eq!(de.decimal, ',');
        assert_eq!(de.grouping, '.');
        assert!(de.symbol_after);

        // No minor unit for yen.
        assert_eq!(number_format_for("ja_JP").currency_decimals, 0);

        // An unlisted locale falls back rather than panicking.
        assert_eq!(number_format_for("xx_YY").currency_code, "USD");
    }

    #[test]
    fn current_locale_defaults_then_tracks_writes() {
        set_current_locale("");
        assert_eq!(current_locale(), DEFAULT_LOCALE);
        set_current_locale("de_DE");
        assert_eq!(current_locale(), "de_DE");
        set_current_locale("");
    }
}
