//! `#[derive(Object)]`: emits the static MetaObject for a struct.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Error, Fields};

use crate::common::type_ref;

pub fn expand(input: DeriveInput) -> Result<TokenStream, Error> {
    let ident = &input.ident;
    if !input.generics.params.is_empty() {
        return Err(Error::new_spanned(
            &input.generics,
            "#[derive(Object)] does not support generic types",
        ));
    }

    let parent = parent_expr(&input)?;
    let properties = property_descriptors(&input)?;

    Ok(quote! {
        #[automatically_derived]
        impl ::rt_core::meta::Object for #ident {
            fn meta() -> &'static ::rt_core::meta::MetaObject {
                static META: ::rt_core::meta::MetaObject = ::rt_core::meta::MetaObject {
                    type_name: stringify!(#ident),
                    parent: #parent,
                    signals: <#ident as ::rt_core::meta::ObjectInputs>::SIGNALS,
                    slots: <#ident as ::rt_core::meta::ObjectInputs>::SLOTS,
                    properties: &[#(#properties),*],
                };
                &META
            }
        }
    })
}

/// `#[object(parent = Path)]` -> `Some(<Path as Object>::meta)` thunk, or `None`.
fn parent_expr(input: &DeriveInput) -> Result<TokenStream, Error> {
    let mut found = None;
    for attr in input
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("object"))
    {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("parent") {
                let path: syn::Path = meta.value()?.parse()?;
                if path
                    .segments
                    .last()
                    .is_some_and(|seg| !matches!(seg.arguments, syn::PathArguments::None))
                {
                    return Err(Error::new_spanned(
                        &path,
                        "parent must be a plain path to a non-generic #[derive(Object)] type",
                    ));
                }
                found = Some(quote!(Some(<#path as ::rt_core::meta::Object>::meta)));
                Ok(())
            } else {
                Err(Error::new_spanned(
                    meta.path,
                    "unknown #[object] item; expected `parent = Type`",
                ))
            }
        })?;
    }
    Ok(found.unwrap_or_else(|| quote!(None)))
}

/// `#[property]` fields -> PropertyDescriptor expressions, in declaration order.
fn property_descriptors(input: &DeriveInput) -> Result<Vec<TokenStream>, Error> {
    let fields = match &input.data {
        Data::Struct(data) => &data.fields,
        _ => {
            return Err(Error::new_spanned(
                input,
                "#[derive(Object)] requires a struct",
            ));
        }
    };
    let named = match fields {
        Fields::Named(named) => named,
        Fields::Unit | Fields::Unnamed(_) => return Ok(Vec::new()),
    };

    let mut descriptors = Vec::new();
    for field in &named.named {
        if !field
            .attrs
            .iter()
            .any(|attr| attr.path().is_ident("property"))
        {
            continue;
        }
        let name = field.ident.as_ref().expect("named field");
        let ty = type_ref(&field.ty);
        descriptors.push(quote! {
            ::rt_core::meta::PropertyDescriptor {
                name: stringify!(#name),
                ty: #ty,
            }
        });
    }
    Ok(descriptors)
}
