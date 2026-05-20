//! Oracle for `engine-i18n`: the shipped en-US corpus must parse, and plural
//! and gender selection must resolve correctly against it (spec II.6).

use engine_i18n::{FluentBundle, LocaleId, LocaleRegistry, Translator, tr};

/// The shipped default-locale corpus, compiled into the test binary.
const EN_US: &str = include_str!("../i18n/en-US/engine.ftl");

fn en_us_translator() -> Translator {
    let locale = LocaleId::parse("en-US").unwrap();
    let mut bundle = FluentBundle::new(locale.clone());
    let count = bundle.add_ftl(EN_US).expect("the en-US corpus parses");
    assert!(
        count >= 10,
        "expected the full corpus, got {count} messages"
    );
    let mut translator = Translator::new(LocaleRegistry::new(locale));
    translator.add_bundle(bundle);
    translator
}

#[test]
fn the_shipped_corpus_parses_and_round_trips() {
    // Parsing the corpus twice into independent bundles must agree — the
    // parse is deterministic, so a recompiled `.locale` table is stable.
    let locale = LocaleId::parse("en-US").unwrap();
    let mut a = FluentBundle::new(locale.clone());
    let mut b = FluentBundle::new(locale);
    let na = a.add_ftl(EN_US).unwrap();
    let nb = b.add_ftl(EN_US).unwrap();
    assert_eq!(na, nb);
    for id in ["app-title", "save-button", "welcome", "entities-selected"] {
        assert!(a.has_message(id) && b.has_message(id), "missing {id}");
    }
}

#[test]
fn plain_and_interpolated_messages_resolve() {
    let t = en_us_translator();
    assert_eq!(tr!(t, "app-title"), "Engine");
    assert_eq!(
        tr!(t, "welcome", name = "Ada"),
        "Welcome to the engine, Ada!"
    );
}

#[test]
fn plural_selection_resolves_every_arm() {
    let t = en_us_translator();
    assert_eq!(
        tr!(t, "entities-selected", count = 0),
        "No entities selected"
    );
    assert_eq!(tr!(t, "entities-selected", count = 1), "1 entity selected");
    assert_eq!(
        tr!(t, "entities-selected", count = 7),
        "7 entities selected"
    );
    // Grouping applies inside the resolved variant.
    assert_eq!(
        tr!(t, "entities-selected", count = 12_000),
        "12,000 entities selected"
    );

    assert_eq!(tr!(t, "assets-imported", count = 1), "Imported one asset");
    assert_eq!(tr!(t, "assets-imported", count = 3), "Imported 3 assets");
}

#[test]
fn gender_selection_resolves_every_arm() {
    let t = en_us_translator();
    assert_eq!(
        tr!(t, "player-finished", gender = "male"),
        "He finished the level"
    );
    assert_eq!(
        tr!(t, "player-finished", gender = "female"),
        "She finished the level"
    );
    assert_eq!(
        tr!(t, "player-finished", gender = "unspecified"),
        "They finished the level"
    );
}

#[test]
fn an_unknown_locale_falls_back_to_en_us() {
    let mut t = en_us_translator();
    // Switch to a locale with no bundle; the chain falls back to en-US.
    t.set_locale(LocaleId::parse("ja-JP").unwrap());
    assert_eq!(tr!(t, "save-button"), "Save");
}
