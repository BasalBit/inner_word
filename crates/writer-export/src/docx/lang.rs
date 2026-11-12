//! The proofing language: `w:lang`.
//!
//! A document whose spelling language is fixed
//! ([`DocumentSettings::spelling_language`](writer_core::DocumentSettings::spelling_language))
//! says so in the exported file, so Word, Pages and LibreOffice check it in
//! that language instead of the reader's own. A document that follows the
//! app's setting, or whose language is detected per paragraph, says
//! nothing: the reader then decides, as before.

use writer_core::Document;

use crate::ExportSource;

/// The region Word's proofing tools are named after for a language that
/// the system's spell checker offers without one (`de`, `fr`, `en`).
const DEFAULT_REGIONS: &[(&str, &str)] = &[
    ("af", "ZA"),
    ("ar", "SA"),
    ("bg", "BG"),
    ("ca", "ES"),
    ("cs", "CZ"),
    ("cy", "GB"),
    ("da", "DK"),
    ("de", "DE"),
    ("el", "GR"),
    ("en", "US"),
    ("es", "ES"),
    ("et", "EE"),
    ("eu", "ES"),
    ("fa", "IR"),
    ("fi", "FI"),
    ("fr", "FR"),
    ("ga", "IE"),
    ("gl", "ES"),
    ("he", "IL"),
    ("hi", "IN"),
    ("hr", "HR"),
    ("hu", "HU"),
    ("hy", "AM"),
    ("id", "ID"),
    ("is", "IS"),
    ("it", "IT"),
    ("ja", "JP"),
    ("ka", "GE"),
    ("kk", "KZ"),
    ("ko", "KR"),
    ("lt", "LT"),
    ("lv", "LV"),
    ("mk", "MK"),
    ("ms", "MY"),
    ("nb", "NO"),
    ("nl", "NL"),
    ("nn", "NO"),
    ("pa", "IN"),
    ("pl", "PL"),
    ("pt", "PT"),
    ("ro", "RO"),
    ("ru", "RU"),
    ("sk", "SK"),
    ("sl", "SI"),
    ("sq", "AL"),
    ("sv", "SE"),
    ("te", "IN"),
    ("th", "TH"),
    ("tr", "TR"),
    ("uk", "UA"),
    ("ur", "PK"),
    ("vi", "VN"),
];

/// Languages written from right to left: their tag goes into `w:bidi`.
const BIDI: &[&str] = &["ar", "dv", "fa", "he", "ps", "sd", "syr", "ug", "ur", "yi"];
/// East Asian languages: their tag goes into `w:eastAsia`.
const EAST_ASIAN: &[&str] = &["ja", "ko", "zh"];

/// The language tag Word expects (`en-GB`, `pt-BR`, `de-DE`) for an
/// identifier of the system's spell checker (`en_GB`, `pt_BR`, `de`).
///
/// The identifier is read as language, script and region, separated by `_`
/// or `-`; anything after them (a variant, a `@collation`) is dropped. A
/// language without a region gets the one its proofing tools are usually
/// named after (`en` is `en-US`, `de` is `de-DE`, `nb` is `nb-NO`), because
/// Word does not check a bare `de`; a language this table does not know
/// stays bare. Chinese is named by region (`zh_Hans` is `zh-CN`, `zh_Hant`
/// is `zh-TW`), and Najdi Arabic (`ars`), which Word does not know, is
/// Arabic. `None` when the identifier does not start with a language code.
///
/// ```
/// use writer_export::docx::language_tag;
/// assert_eq!(language_tag("en_GB").as_deref(), Some("en-GB"));
/// assert_eq!(language_tag("pt_BR").as_deref(), Some("pt-BR"));
/// assert_eq!(language_tag("de").as_deref(), Some("de-DE"));
/// assert_eq!(language_tag("auto"), None);
/// ```
pub fn language_tag(identifier: &str) -> Option<String> {
    let identifier = identifier.trim();
    // Everything from `@` or `.` on is not part of the language.
    let identifier = identifier.split(['@', '.']).next().unwrap_or_default();
    let mut subtags = identifier.split(['_', '-']);
    let language = subtags.next()?.to_ascii_lowercase();
    let is_language =
        (2..=3).contains(&language.len()) && language.bytes().all(|b| b.is_ascii_lowercase());
    if !is_language || language == "und" {
        return None;
    }
    let mut script = None;
    let mut region = None;
    for subtag in subtags {
        let letters = subtag.bytes().all(|b| b.is_ascii_alphabetic());
        let digits = subtag.bytes().all(|b| b.is_ascii_digit());
        if script.is_none() && region.is_none() && subtag.len() == 4 && letters {
            let lower = subtag.to_ascii_lowercase();
            script = Some(format!(
                "{}{}",
                lower[..1].to_ascii_uppercase(),
                &lower[1..]
            ));
        } else if region.is_none()
            && ((subtag.len() == 2 && letters) || (subtag.len() == 3 && digits))
        {
            region = Some(subtag.to_ascii_uppercase());
        } else {
            // A variant or anything else: the tag ends here.
            break;
        }
    }
    let mut language = language;
    if language == "ars" {
        language = "ar".into();
        region = None;
    }
    if language == "zh" {
        // Word names Chinese by region, not by script.
        let region = region.unwrap_or_else(|| match script.as_deref() {
            Some("Hant") => "TW".into(),
            _ => "CN".into(),
        });
        return Some(format!("zh-{region}"));
    }
    let region = region.or_else(|| {
        // The default region belongs to the default script only.
        script.is_none().then(|| {
            DEFAULT_REGIONS
                .iter()
                .find(|(l, _)| *l == language)
                .map(|(_, r)| r.to_string())
        })?
    });
    let mut tag = language;
    for subtag in [script, region].into_iter().flatten() {
        tag.push('-');
        tag.push_str(&subtag);
    }
    Some(tag)
}

/// A proofing language as it is written into run properties.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Lang {
    tag: String,
}

impl Lang {
    /// The fixed language of `doc`; `None` when it has none (it follows the
    /// app's setting, or the language is detected automatically) or the
    /// identifier is not a language.
    pub(crate) fn of(doc: &Document) -> Option<Lang> {
        let identifier = doc.settings().spelling_language.as_ref()?.identifier()?;
        Lang::from_identifier(identifier)
    }

    /// The language a spell checker's identifier names, if it names one.
    pub(crate) fn from_identifier(identifier: &str) -> Option<Lang> {
        language_tag(identifier).map(|tag| Lang { tag })
    }

    /// The language every part of `source` is written in, when they all
    /// name the same one: the styles carry it then. Otherwise each part
    /// with a language of its own says so on its runs.
    pub(crate) fn common(source: &ExportSource<'_>) -> Option<Lang> {
        let mut parts = source.parts().iter().map(|p| Lang::of(p.document));
        let first = parts.next()??;
        parts.all(|l| l.as_ref() == Some(&first)).then_some(first)
    }

    /// `<w:lang …/>`, a child of `w:rPr`. Word keeps three languages per
    /// run, one per kind of script, and uses the one the characters belong
    /// to: the tag goes where its language is looked up.
    pub(crate) fn xml(&self) -> String {
        let language = self.tag.split('-').next().unwrap_or_default();
        let attribute = if BIDI.contains(&language) {
            "w:bidi"
        } else if EAST_ASIAN.contains(&language) {
            "w:eastAsia"
        } else {
            "w:val"
        };
        format!("<w:lang {attribute}=\"{}\"/>", self.tag)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags() {
        let cases: &[(&str, Option<&str>)] = &[
            // What the Mac's spell checker offers.
            ("en_GB", Some("en-GB")),
            ("en", Some("en-US")),
            ("en_AU", Some("en-AU")),
            ("en_JP", Some("en-JP")),
            ("pt_BR", Some("pt-BR")),
            ("pt_PT", Some("pt-PT")),
            ("de", Some("de-DE")),
            ("fr", Some("fr-FR")),
            ("nb", Some("nb-NO")),
            ("nn", Some("nn-NO")),
            ("ar", Some("ar-SA")),
            ("ars", Some("ar-SA")),
            ("he", Some("he-IL")),
            ("ko", Some("ko-KR")),
            ("te", Some("te-IN")),
            // Other spellings of the same.
            ("en-gb", Some("en-GB")),
            ("EN_gb", Some("en-GB")),
            ("  fr_CA ", Some("fr-CA")),
            ("de_CH@collation=phonebook", Some("de-CH")),
            ("en_US.UTF-8", Some("en-US")),
            ("es_419", Some("es-419")),
            ("sr_Latn_RS", Some("sr-Latn-RS")),
            ("sr-latn", Some("sr-Latn")),
            ("zh_Hans", Some("zh-CN")),
            ("zh-Hant", Some("zh-TW")),
            ("zh_Hant_HK", Some("zh-HK")),
            ("zh", Some("zh-CN")),
            ("ca_ES_valencia", Some("ca-ES")),
            // A language without a usual region stays bare.
            ("la", Some("la")),
            ("haw", Some("haw")),
            // Not a language.
            ("", None),
            ("auto", None),
            ("e", None),
            ("x_YY", None),
            ("und", None),
            ("12", None),
            ("_GB", None),
            ("Multilingual", None),
        ];
        for (identifier, expected) in cases {
            assert_eq!(
                language_tag(identifier).as_deref(),
                *expected,
                "{identifier:?}"
            );
        }
    }

    #[test]
    fn the_attribute_follows_the_script() {
        let xml = |tag: &str| Lang { tag: tag.into() }.xml();
        assert_eq!(xml("en-GB"), "<w:lang w:val=\"en-GB\"/>");
        assert_eq!(xml("he-IL"), "<w:lang w:bidi=\"he-IL\"/>");
        assert_eq!(xml("ar-SA"), "<w:lang w:bidi=\"ar-SA\"/>");
        assert_eq!(xml("ko-KR"), "<w:lang w:eastAsia=\"ko-KR\"/>");
        assert_eq!(xml("zh-TW"), "<w:lang w:eastAsia=\"zh-TW\"/>");
    }
}
