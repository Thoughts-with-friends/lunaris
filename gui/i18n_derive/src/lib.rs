use proc_macro::TokenStream;
use quote::quote;

/// Generate i18n helper implementations from enum variant doc comments.
///
/// # Generated
///
/// - `I18nKey::ALL`
/// - `I18nKey::default_eng()`
///
/// # Rules
///
/// - Each variant must have a `///` doc comment.
/// - Multi-line docs are joined with `\n`.
/// - Only supports field-less enums.
///
/// # Example
///
/// Written as `text` rather than `rust`: this is a sketch of a caller and of
/// what the macro emits, not a compilable unit — `I18nKey` is the caller's own
/// type and does not exist here.
///
/// ```text
/// #[derive(I18n)]
/// enum I18nKey {
///     /// Execute
///     #[i18n(ja = "実行")]
///     ExecuteButton,
///
///     /// Cancel
///     #[i18n(ja = "キャンセル")]
///     CancelButton,
///
///     /// Internal invalid key
///     Invalid,
/// }
/// ```
///
/// Expands roughly to:
///
/// ```text
/// impl I18nKey {
///     pub const ALL: &'static [Self] = &[
///         Self::ExecuteButton,
///         Self::CancelButton,
///         Self::Invalid
///     ];
///
///     pub const UNTRANSLATED: &'static [Self] = &[Self::Invalid];
///
///     pub const fn default_eng(&self) -> &'static str {
///         match self {
///             Self::ExecuteButton => "Execute",
///             Self::CancelButton => "Cancel",
///             Self::Invalid => "Internal invalid key",
///         }
///     }
///
///     pub const fn default_jpn(&self) -> &'static str {
///         match self {
///             Self::ExecuteButton => "実行",
///             Self::CancelButton => "キャンセル",
///             // No `ja`, so the English stands in.
///             Self::Invalid => "Internal invalid key",
///         }
///     }
/// }
/// ```
///
/// # Panics
/// If invalid syntax as Rust
#[proc_macro_derive(I18n, attributes(i18n))]
pub fn derive_i18n(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);

    let enum_name = &input.ident;

    let syn::Data::Enum(data_enum) = &input.data else {
        panic!("I18n only supports enums");
    };

    let mut doc_lines = vec![
        //
        "Generated from enum variant doc comments.\n".to_string(),
        "# Keys\n\n".to_string(),
        "| Key | Default |".to_string(),
        "|-----|---------|".to_string(),
    ];

    let mut variants = Vec::new();
    let mut match_arms = Vec::new();
    let mut jpn_arms = Vec::new();
    let mut untranslated = Vec::new();

    for variant in &data_enum.variants {
        let ident = &variant.ident;

        variants.push(quote! {
            Self::#ident
        });

        let mut docs = Vec::new();
        // `#[i18n(ja = "...")]`, if the variant has been translated.
        let mut japanese: Option<String> = None;

        for attr in &variant.attrs {
            if attr.path().is_ident("i18n") {
                attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("ja") {
                        let value: syn::LitStr = meta.value()?.parse()?;
                        japanese = Some(value.value());
                        return Ok(());
                    }
                    Err(meta.error("expected `ja = \"...\"`"))
                })
                .unwrap_or_else(|error| panic!("bad #[i18n(...)] on `{ident}`: {error}"));
            }
        }

        for attr in &variant.attrs {
            if attr.path().is_ident("doc")
                && let syn::Meta::NameValue(meta) = &attr.meta
                && let syn::Expr::Lit(expr) = &meta.value
                && let syn::Lit::Str(lit) = &expr.lit
            {
                docs.push(lit.value().trim().to_string());
            }
        }

        if docs.is_empty() {
            panic!("missing doc comment for variant `{ident}`");
        }

        let joined = docs.join("\n");

        match_arms.push(quote! {
            Self::#ident => #joined
        });

        // An untranslated key renders its English rather than a placeholder:
        // a half-translated UI is usable, and a UI full of `??` is not.
        let jpn = japanese.clone().unwrap_or_else(|| joined.clone());
        jpn_arms.push(quote! {
            Self::#ident => #jpn
        });
        if japanese.is_none() {
            untranslated.push(quote! { Self::#ident });
        }

        fn escape_md_table_cell(s: &str) -> String {
            s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('|', "\\|")
        }
        doc_lines.push(format!("| `{ident}` | {} |", escape_md_table_cell(&joined)));
    }

    let generated_doc = doc_lines.join("\n");
    quote! {
        impl #enum_name {
            pub const ALL: &'static [Self] = &[
                #(#variants),*
            ];

            #[doc = #generated_doc]
            pub const fn default_eng(&self) -> &'static str {
                match self {
                    #(#match_arms),*
                }
            }

            /// The built-in Japanese text, falling back to the English for any
            /// key that has no `#[i18n(ja = "...")]` yet.
            pub const fn default_jpn(&self) -> &'static str {
                match self {
                    #(#jpn_arms),*
                }
            }

            /// Every key that has no Japanese text of its own.
            ///
            /// Exists so the gap is a number the UI can report rather than
            /// something a reader has to notice, and so a test can assert on
            /// it.
            pub const UNTRANSLATED: &'static [Self] = &[
                #(#untranslated),*
            ];
        }
    }
    .into()
}
