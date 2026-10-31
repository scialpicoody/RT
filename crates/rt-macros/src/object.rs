//! `#[derive(Object)]`: emits the static MetaObject for a struct.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Error, Fields, Ident, Type};

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
    let properties = property_fields(&input)?;

    let descriptors = properties.iter().map(|(name, inner)| {
        let ty = type_ref(inner);
        quote! {
            ::rt_core::meta::PropertyDescriptor {
                name: stringify!(#name),
                ty: #ty,
            }
        }
    });

    // Property-change signal tokens sit after the declared signals in index
    // space: SIGNALS.len() + declaration offset.
    let mut accessors = TokenStream::new();
    for (offset, (name, inner)) in properties.iter().enumerate() {
        let getter = name;
        let setter = Ident::new(&format!("set_{name}"), name.span());
        let token = Ident::new(&format!("{}_CHANGED", name.to_string().to_uppercase()), name.span());
        let doc = format!("Change token for the `{name}` property.");
        accessors.extend(quote! {
            #[doc = #doc]
            pub const #token: ::rt_core::signal::SignalId<Self, #inner> =
                ::rt_core::signal::SignalId::new(
                    (<Self as ::rt_core::meta::ObjectInputs>::SIGNALS.len() + #offset) as u32,
                );

            pub fn #getter(&self) -> #inner {
                self.#name.get()
            }

            pub fn #setter(&self, value: #inner) {
                self.#name.set(value);
            }
        });
    }

    let takes = properties.iter().enumerate().map(|(offset, (name, _))| {
        quote! {
            if self.#name.take_dirty() {
                changes.push((
                    (<Self as ::rt_core::meta::ObjectInputs>::SIGNALS.len() + #offset) as u32,
                    ::std::boxed::Box::new(self.#name.get())
                        as ::std::boxed::Box<dyn ::rt_core::property::CloneAny>,
                ));
            }
        }
    });

    Ok(quote! {
        #[automatically_derived]
        impl ::rt_core::property::PropertyInputs for #ident {
            fn take_dirty_changes(
                &mut self,
            ) -> ::std::vec::Vec<(u32, ::std::boxed::Box<dyn ::rt_core::property::CloneAny>)> {
                let mut changes = ::std::vec::Vec::new();
                #(#takes)*
                changes
            }
        }

        #[automatically_derived]
        impl ::rt_core::meta::Object for #ident {
            fn meta() -> &'static ::rt_core::meta::MetaObject {
                static META: ::rt_core::meta::MetaObject = ::rt_core::meta::MetaObject {
                    type_name: stringify!(#ident),
                    parent: #parent,
                    signals: <#ident as ::rt_core::meta::ObjectInputs>::SIGNALS,
                    slots: <#ident as ::rt_core::meta::ObjectInputs>::SLOTS,
                    properties: &[#(#descriptors),*],
                    slot_invokers: <#ident as ::rt_core::meta::ObjectInputs>::SLOT_INVOKERS,
                };
                &META
            }
        }

        #[allow(clippy::new_ret_no_self)]
        impl #ident {
            #accessors
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

/// `#[property]` fields as `(name, inner type)` in declaration order.
/// The field type must be `Property<T>`; the descriptor reports `T`.
fn property_fields(input: &DeriveInput) -> Result<Vec<(Ident, Type)>, Error> {
    let fields = match &input.data {
        Data::Struct(data) => &data.fields,
        _ => {
            return Err(Error::new_spanned(
                input,
                "#[derive(Object)] requires a struct",
            ));
        }
    };
    let Fields::Named(named) = fields else {
        return Ok(Vec::new());
    };

    let mut properties = Vec::new();
    for field in &named.named {
        if !field
            .attrs
            .iter()
            .any(|attr| attr.path().is_ident("property"))
        {
            continue;
        }
        let name = field.ident.clone().expect("named field");
        let inner = inner_property_type(&field.ty)?;
        properties.push((name, inner));
    }
    Ok(properties)
}

/// Unwraps `Property<T>` to `T`.
fn inner_property_type(ty: &Type) -> Result<Type, Error> {
    let Type::Path(type_path) = ty else {
        return Err(Error::new_spanned(
            ty,
            "#[property] fields must be `Property<T>`",
        ));
    };
    let last = type_path.path.segments.last().ok_or_else(|| {
        Error::new_spanned(ty, "#[property] fields must be `Property<T>`")
    })?;
    if last.ident != "Property" {
        return Err(Error::new_spanned(
            ty,
            "#[property] fields must be `Property<T>`",
        ));
    }
    let syn::PathArguments::AngleBracketed(args) = &last.arguments else {
        return Err(Error::new_spanned(
            ty,
            "#[property] fields must be `Property<T>`",
        ));
    };
    let first = args.args.first().ok_or_else(|| {
        Error::new_spanned(ty, "#[property] fields must be `Property<T>`")
    })?;
    match first {
        syn::GenericArgument::Type(inner) => Ok(inner.clone()),
        other => Err(Error::new_spanned(
            other,
            "#[property] fields must be `Property<T>`",
        )),
    }
}
