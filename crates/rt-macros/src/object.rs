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

    let parent = parent_path(&input)?;
    let parent_field = parent_value_field(&input, parent.as_ref())?;
    let parent_thunk = parent.as_ref().map_or_else(
        || quote!(None),
        |path| quote!(Some(<#path as ::rt_core::meta::Object>::meta)),
    );
    let properties = property_fields(&input)?;
    let children = children_fields(&input)?;

    // Owned-children cascade hook: the parent value field's own children
    // stay reachable through the child's hook, so destroying a child object
    // also cascades into inherited children.
    let parent_children = parent_field
        .as_ref()
        .map(|field| quote!(out.extend(::rt_core::property::PropertyInputs::child_handles(&self.#field));));
    let child_takes = children.iter().map(|field| {
        quote!(out.extend(self.#field.iter().copied());)
    });

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
    // Inherited properties join the same dirty-report in the merged property
    // space: the parent value field's changes arrive in the parent's merged
    // index space and shift past the child's whole signal+property space.
    let parent_takes = parent_field.as_ref().map(|field| {
        let property_space = properties.len();
        quote! {
            changes.extend(
                self.#field
                    .take_dirty_changes()
                    .into_iter()
                    .map(|(index, value)| {
                        (
                            index
                                + ((<Self as ::rt_core::meta::ObjectInputs>::SIGNALS.len()
                                    + #property_space)
                                    as u32),
                            value,
                        )
                    }),
            );
        }
    });

    // By-name property fallbacks: names the child does not own route to the
    // parent value field, which serves its own inherited members the same
    // way — one hop per level, nearest ancestor first.
    let parent_fallbacks = match parent_field.as_ref() {
        Some(field) => quote! {
            _ => self.#field.is_dirty(name),
        },
        None => quote! {
            _ => ::std::option::Option::None,
        },
    };
    let parent_get_fallbacks = match parent_field.as_ref() {
        Some(field) => quote! {
            _ => self.#field.get_property(name),
        },
        None => quote! {
            _ => ::std::option::Option::None,
        },
    };
    let parent_set_fallbacks = match parent_field.as_ref() {
        Some(field) => quote! {
            _ => self.#field.set_property(name, value),
        },
        None => quote! {
            _ => ::std::result::Result::Err(value),
        },
    };

    // By-name accessors: one arm per `#[property]` field, keyed by the
    // literal field name so the erased dynamic path can serve them.
    let name_lits = properties.iter().map(|(name, _)| {
        proc_macro2::Literal::string(&name.to_string())
    });
    let get_arms = properties.iter().zip(name_lits.clone()).map(|((name, _), lit)| {
        quote! {
            #lit => Some(::std::boxed::Box::new(self.#name.get())),
        }
    });
    let dirty_arms = properties.iter().zip(name_lits.clone()).map(|((name, _), lit)| {
        quote! {
            #lit => ::std::option::Option::Some(self.#name.peek_dirty()),
        }
    });
    let set_arms = properties.iter().zip(name_lits).map(|((name, inner), lit)| {
        quote! {
            #lit => {
                let value = *value
                    .downcast::<#inner>()
                    .expect("rt: by-name property type mismatch");
                self.#name.set(value);
                ::std::result::Result::Ok(())
            }
        }
    });

    let parent_route = parent_field
        .as_ref()
        .map(|field| {
            let path = parent.as_ref().expect("parent field implies parent attr");
            quote! {
            Some({
                fn route(
                    recv: &mut dyn ::core::any::Any,
                    up: u8,
                ) -> &mut dyn ::core::any::Any {
                    let recv = recv
                        .downcast_mut::<#ident>()
                        .expect("rt: receiver type mismatch");
                    match up {
                        0 => recv,
                        1 => &mut recv.#field,
                        n => {
                            let parent_route = <#path as ::rt_core::meta::Object>::meta()
                                .parent_route
                                .expect("rt: parent chain has no route");
                            parent_route(&mut recv.#field, n - 1)
                        }
                    }
                }
                route
            })
        }
    });
    let parent_route = parent_route.unwrap_or_else(|| quote!(None));

    Ok(quote! {
        #[automatically_derived]
        impl ::rt_core::property::PropertyInputs for #ident {
            fn take_dirty_changes(
                &mut self,
            ) -> ::std::vec::Vec<(u32, ::std::boxed::Box<dyn ::rt_core::property::CloneAny>)> {
                let mut changes = ::std::vec::Vec::new();
                #(#takes)*
                #parent_takes
                changes
            }

            fn child_handles(
                &self,
            ) -> ::std::vec::Vec<::rt_core::Handle> {
                let mut out = ::std::vec::Vec::new();
                #(#child_takes)*
                #parent_children
                out
            }

            fn is_dirty(&self, name: &str) -> ::std::option::Option<bool> {
                match name {
                    #(#dirty_arms)*
                    #parent_fallbacks
                }
            }

            fn get_property(
                &self,
                name: &str,
            ) -> ::std::option::Option<::std::boxed::Box<dyn ::core::any::Any>> {
                match name {
                    #(#get_arms)*
                    #parent_get_fallbacks
                }
            }

            fn set_property(
                &mut self,
                name: &str,
                value: ::std::boxed::Box<dyn ::core::any::Any>,
            ) -> ::std::result::Result<(), ::std::boxed::Box<dyn ::core::any::Any>> {
                match name {
                    #(#set_arms)*
                    #parent_set_fallbacks
                }
            }
        }

        #[automatically_derived]
        impl ::rt_core::meta::Object for #ident {
            fn meta() -> &'static ::rt_core::meta::MetaObject {
                static META: ::rt_core::meta::MetaObject = ::rt_core::meta::MetaObject {
                    type_name: stringify!(#ident),
                    parent: #parent_thunk,
                    signals: <#ident as ::rt_core::meta::ObjectInputs>::SIGNALS,
                    slots: <#ident as ::rt_core::meta::ObjectInputs>::SLOTS,
                    properties: &[#(#descriptors),*],
                    slot_invokers: <#ident as ::rt_core::meta::ObjectInputs>::SLOT_INVOKERS,
                    parent_route: #parent_route,
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
fn parent_path(input: &DeriveInput) -> Result<Option<syn::Path>, Error> {
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
                found = Some(path);
                Ok(())
            } else {
                Err(Error::new_spanned(
                    meta.path,
                    "unknown #[object] item; expected `parent = Type`",
                ))
            }
        })?;
    }
    Ok(found)
}

/// The parent value field: the exactly-one field whose type is the parent
/// type. Meta links alone carry no state — a derived object stores its
/// parent's value in that field, so inherited members operate on real state.
fn parent_value_field(
    input: &DeriveInput,
    parent: Option<&syn::Path>,
) -> Result<Option<Ident>, Error> {
    let Some(parent) = parent else {
        return Ok(None);
    };
    let base_name = parent
        .segments
        .last()
        .map(|segment| segment.ident.to_string())
        .expect("non-empty path");

    let Data::Struct(data) = &input.data else {
        return Err(Error::new_spanned(
            input,
            "#[derive(Object)] requires a struct",
        ));
    };
    let Fields::Named(named) = &data.fields else {
        return Err(Error::new_spanned(
            input,
            format!(
                "#[object(parent = ..)] requires exactly one field of type `{base_name}` — the parent value field — but the struct has no named fields"
            ),
        ));
    };
    let mut matches = named
        .named
        .iter()
        .filter(|field| last_type_ident(&field.ty) == base_name)
        .map(|field| field.ident.clone().expect("named field"));
    let Some(field) = matches.next() else {
        return Err(Error::new_spanned(
            input,
            format!(
                "#[object(parent = {base_name})] requires exactly one field of type `{base_name}` — the parent value field — but none exists"
            ),
        ));
    };
    if matches.next().is_some() {
        return Err(Error::new_spanned(
            input,
            format!(
                "#[object(parent = {base_name})] requires exactly one field of type `{base_name}` — the parent value field — but several exist"
            ),
        ));
    }
    Ok(Some(field))
}

/// The final path segment's identifier of a syntactic type, if any.
fn last_type_ident(ty: &Type) -> String {
    match ty {
        Type::Path(path) => path
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string())
            .unwrap_or_default(),
        _ => String::new(),
    }
}

/// `#[object(children)]` fields in declaration order. The field type must be
/// `Vec<rt_core::Handle>` — the lifecycle cascade (ticket #31) destroys every
/// handle in the collection depth-first.
fn children_fields(input: &DeriveInput) -> Result<Vec<Ident>, Error> {
    let Data::Struct(data) = &input.data else {
        return Err(Error::new_spanned(
            input,
            "#[derive(Object)] requires a struct",
        ));
    };
    let Fields::Named(named) = &data.fields else {
        return Ok(Vec::new());
    };
    let mut children = Vec::new();
    for field in &named.named {
        let is_children = field
            .attrs
            .iter()
            .any(|attr| attr.path().is_ident("object"))
            && {
                let mut found = false;
                for attr in field.attrs.iter().filter(|a| a.path().is_ident("object")) {
                    attr.parse_nested_meta(|meta| {
                        if meta.path.is_ident("children") {
                            found = true;
                            Ok(())
                        } else {
                            Err(Error::new_spanned(
                                meta.path,
                                "unknown #[object] field item; expected `children`",
                            ))
                        }
                    })?;
                }
                found
            };
        if !is_children {
            continue;
        }
        if !is_handle_vec(&field.ty) {
            return Err(Error::new_spanned(
                &field.ty,
                "#[object(children)] fields must be Vec<rt_core::Handle>",
            ));
        }
        children.push(field.ident.clone().expect("named field"));
    }
    Ok(children)
}

/// Syntactic check: `Vec<Handle>` (any path spelling ending in `Handle`).
fn is_handle_vec(ty: &Type) -> bool {
    let Type::Path(path) = ty else {
        return false;
    };
    let Some(last) = path.path.segments.last() else {
        return false;
    };
    if last.ident != "Vec" {
        return false;
    }
    let syn::PathArguments::AngleBracketed(args) = &last.arguments else {
        return false;
    };
    match args.args.first() {
        Some(syn::GenericArgument::Type(inner)) => last_type_ident(inner) == "Handle",
        _ => false,
    }
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

#[cfg(test)]
mod parent_field_tests {
    use super::*;

    fn expand_struct(source: &str) -> Result<TokenStream, Error> {
        expand(syn::parse_str::<DeriveInput>(source).expect("valid struct"))
    }

    #[test]
    fn missing_parent_value_field_is_a_clear_derive_error() {
        let error = expand_struct(
            "#[derive(Object)] #[object(parent = Base)] struct Child;",
        )
        .expect_err("missing parent value field");
        assert!(error.to_string().contains("the parent value field"));
        assert!(error.to_string().contains("`Base`"));
    }

    #[test]
    fn ambiguous_parent_value_field_is_a_clear_derive_error() {
        let error = expand_struct(
            "#[derive(Object)] #[object(parent = Base)] struct Child { one: Base, two: Base }",
        )
        .expect_err("ambiguous parent value field");
        assert!(error.to_string().contains("several exist"));
    }

    #[test]
    fn children_field_must_be_a_handle_vec() {
        let error = expand_struct(
            "#[derive(Object)] struct N { #[object(children)] kids: Vec<u32> }",
        )
        .expect_err("non-handle children field");
        assert!(
            error
                .to_string()
                .contains("#[object(children)] fields must be Vec<rt_core::Handle>")
        );

        let error = expand_struct("#[derive(Object)] struct N { #[object(children)] kids: u32 }")
            .expect_err("non-vec children field");
        assert!(error.to_string().contains("Vec<rt_core::Handle>"));
    }

    #[test]
    fn unknown_object_field_item_is_rejected() {
        let error = expand_struct("#[derive(Object)] struct N { #[object(kids)] v: u32 }")
            .expect_err("unknown field item");
        assert!(error.to_string().contains("unknown #[object] field item"));
    }
}
