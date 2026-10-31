//! Proc macros for RT: `#[derive(Object)]` with `#[object_impl]` generate a
//! `&'static MetaObject` per type — see ADR 0002.

#![forbid(unsafe_code)]

mod common;
mod object;
mod object_impl;

use proc_macro::TokenStream;

/// Derives `rt_core::Object` for a struct: emits the static MetaObject from
/// `#[property]` fields and the `#[object(parent = ..)]` attribute.
///
/// Every derived type also needs one `#[object_impl]` inherent impl block
/// (it provides the signal/slot descriptors).
#[proc_macro_derive(Object, attributes(object, property))]
pub fn derive_object(input: TokenStream) -> TokenStream {
    match object::expand(syn::parse_macro_input!(input as syn::DeriveInput)) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

/// Harvests `#[signal]`/`#[slot]` methods of an inherent impl into
/// `rt_core::ObjectInputs` descriptors; generates signal method bodies.
#[proc_macro_attribute]
pub fn object_impl(_attr: TokenStream, item: TokenStream) -> TokenStream {
    match object_impl::expand(syn::parse_macro_input!(item as syn::ItemImpl)) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}
