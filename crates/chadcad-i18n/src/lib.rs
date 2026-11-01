//! UI translations, in Fluent files compiled into the binary.
//!
//! To add a language: copy `locales/en.ftl`, translate it, and add one row to
//! [`LANGUAGES`]. The tests fail if any locale is missing a key.

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentArgs, FluentResource};
use unic_langid::LanguageIdentifier;

/// Code, native name and source of every shipped language.
pub const LANGUAGES: [(&str, &str, &str); 4] = [
    ("en", "English", include_str!("../locales/en.ftl")),
    ("es", "Español", include_str!("../locales/es.ftl")),
    ("fr", "Français", include_str!("../locales/fr.ftl")),
    ("de", "Deutsch", include_str!("../locales/de.ftl")),
];

const FALLBACK: &str = "en";

fn bundle(code: &str, source: &str) -> FluentBundle<FluentResource> {
    let id: LanguageIdentifier = code.parse().expect("language codes in LANGUAGES are valid");
    let mut bundle = FluentBundle::new_concurrent(vec![id]);
    // egui renders the bidi isolation marks Fluent adds around arguments as boxes.
    bundle.set_use_isolating(false);
    let resource = FluentResource::try_new(source.to_owned())
        .unwrap_or_else(|(_, errors)| panic!("{code}.ftl does not parse: {errors:?}"));
    bundle.add_resource(resource).expect("locale files have no duplicate keys");
    bundle
}

pub struct I18n {
    code: &'static str,
    bundle: FluentBundle<FluentResource>,
    fallback: FluentBundle<FluentResource>,
}

impl I18n {
    /// Loads `code`, or English when it is not shipped.
    pub fn new(code: &str) -> Self {
        let language = code.split(['-', '_']).next().unwrap_or(code).to_ascii_lowercase();
        let (code, _, source) =
            LANGUAGES.iter().find(|(c, _, _)| *c == language).copied().unwrap_or(LANGUAGES[0]);
        let fallback_source = LANGUAGES[0].2;
        Self { code, bundle: bundle(code, source), fallback: bundle(FALLBACK, fallback_source) }
    }

    /// The language to start in: `CHADCAD_LANG`, then the OS locale.
    pub fn detect() -> Self {
        let wanted = std::env::var("CHADCAD_LANG").ok().or_else(sys_locale::get_locale);
        Self::new(wanted.as_deref().unwrap_or(FALLBACK))
    }

    pub fn code(&self) -> &'static str {
        self.code
    }

    pub fn t(&self, key: &str) -> String {
        self.format(key, None)
    }

    /// Translates `key`, substituting `{ $name }` arguments.
    pub fn t_with(&self, key: &str, args: &[(&str, String)]) -> String {
        let mut fluent = FluentArgs::new();
        for (name, value) in args {
            fluent.set(*name, value.clone());
        }
        self.format(key, Some(&fluent))
    }

    fn format(&self, key: &str, args: Option<&FluentArgs>) -> String {
        [&self.bundle, &self.fallback]
            .into_iter()
            .find_map(|bundle| {
                let pattern = bundle.get_message(key)?.value()?;
                let mut errors = Vec::new();
                Some(bundle.format_pattern(pattern, args, &mut errors).into_owned())
            })
            .unwrap_or_else(|| key.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use fluent_bundle::FluentResource;
    use fluent_syntax::ast::Entry;

    use super::*;

    fn keys(source: &str) -> BTreeSet<String> {
        let resource = FluentResource::try_new(source.to_owned()).expect("parses");
        resource
            .entries()
            .filter_map(|e| match e {
                Entry::Message(m) => Some(m.id.name.to_owned()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn every_locale_has_exactly_the_english_keys() {
        let english = keys(LANGUAGES[0].2);
        assert!(english.len() > 50);
        for (code, _, source) in LANGUAGES {
            let other = keys(source);
            let missing: Vec<_> = english.difference(&other).collect();
            let extra: Vec<_> = other.difference(&english).collect();
            assert!(missing.is_empty() && extra.is_empty(), "{code}: -{missing:?} +{extra:?}");
        }
    }

    #[test]
    fn looks_up_with_fallbacks() {
        assert_eq!(I18n::new("es").t("menu-file"), "Archivo");
        assert_eq!(I18n::new("fr-CA").code(), "fr");
        assert_eq!(I18n::new("xx").t("menu-file"), "File");
        assert_eq!(I18n::new("de").t("no-such-key"), "no-such-key");
    }

    #[test]
    fn substitutes_arguments_without_isolation_marks() {
        let text = I18n::new("en").t_with("status-exported", &[("path", "a.step".into())]);
        assert!(text.contains("a.step") && text.is_ascii(), "{text:?}");
    }
}
