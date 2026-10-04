use gpui_kit::component::highlighter::{GrammarConfig, LanguageRegistry};

/// Gramáticas que o gpui-kit não traz ou traz sem cor: C# e Swift vêm com `highlights` vazio, e as regras do Kotlin
/// são de outra versão da gramática e não compilam. O teste `every_mapped_language_has_a_grammar_with_color_rules` acusa.
pub(crate) fn register() {
    let registry = LanguageRegistry::singleton();
    for (name, language, highlights) in [
        ("csharp", tree_sitter_c_sharp::LANGUAGE, tree_sitter_c_sharp::HIGHLIGHTS_QUERY),
        ("swift", tree_sitter_swift::LANGUAGE, tree_sitter_swift::HIGHLIGHTS_QUERY),
        ("kotlin", tree_sitter_kotlin_sg::LANGUAGE, tree_sitter_kotlin_sg::HIGHLIGHTS_QUERY),
        // O pacote do Pascal não exporta as regras; cópia de queries/highlights.scm (Isopod/tree-sitter-pascal, MIT).
        ("pascal", tree_sitter_pascal::LANGUAGE, include_str!("pascal_highlights.scm")),
        ("dart", tree_sitter_dart::LANGUAGE, tree_sitter_dart::HIGHLIGHTS_QUERY),
    ] {
        registry.register(name, &GrammarConfig::new(name, tree_sitter::Language::new(language), vec![], highlights, "", ""));
    }
}
