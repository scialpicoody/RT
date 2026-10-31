//! Shared descriptor-emission helpers.

use proc_macro2::TokenStream;
use quote::quote;
use syn::Type;

/// Builds a `TypeRef { name, id }` descriptor expression for a syntactic type.
pub fn type_ref(ty: &Type) -> TokenStream {
    let name = render_type(ty);
    quote! {
        ::rt_core::meta::TypeRef {
            name: #name,
            id: ::core::any::TypeId::of::<#ty>(),
        }
    }
}

/// Renders a type as source-like text: space only between adjacent words.
fn render_type(ty: &Type) -> String {
    use proc_macro2::{Delimiter, TokenTree};

    fn join(stream: TokenStream) -> String {
        let mut out = String::new();
        let mut prev_wordy = false;
        for tree in stream {
            let wordy = matches!(tree, TokenTree::Ident(_) | TokenTree::Literal(_));
            if wordy && prev_wordy {
                out.push(' ');
            }
            match tree {
                TokenTree::Ident(ident) => out.push_str(&ident.to_string()),
                TokenTree::Literal(lit) => out.push_str(&lit.to_string()),
                TokenTree::Punct(punct) => out.push(punct.as_char()),
                TokenTree::Group(group) => {
                    let (open, close) = match group.delimiter() {
                        Delimiter::Parenthesis => ('(', ')'),
                        Delimiter::Bracket => ('[', ']'),
                        Delimiter::Brace => ('{', '}'),
                        Delimiter::None => ('\0', '\0'),
                    };
                    if open != '\0' {
                        out.push(open);
                    }
                    out.push_str(&join(group.stream()));
                    if close != '\0' {
                        out.push(close);
                    }
                }
            }
            prev_wordy = wordy;
        }
        out
    }

    join(quote!(#ty))
}
