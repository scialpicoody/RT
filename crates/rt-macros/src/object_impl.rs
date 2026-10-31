//! `#[object_impl]`: harvests `#[signal]`/`#[slot]` methods into descriptors.

use proc_macro2::TokenStream;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{
    Attribute, Error, FnArg, Ident, ImplItem, ImplItemFn, ItemImpl, Pat, Receiver, ReturnType,
    Signature, Token, Visibility,
};

use crate::common::type_ref;

enum Member {
    Signal,
    Slot,
}

/// One harvested `#[signal]`: its descriptor, payload type, and const name.
struct SignalInfo {
    descriptor: TokenStream,
    const_ident: Ident,
    args_ty: TokenStream,
}

/// A bodyless impl method (`fn f(&self);`), which syn reports as `Verbatim`.
struct MethodDecl {
    attrs: Vec<Attribute>,
    vis: Visibility,
    sig: Signature,
}

impl Parse for MethodDecl {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let attrs = input.call(Attribute::parse_outer)?;
        let vis: Visibility = input.parse()?;
        let sig: Signature = input.parse()?;
        let _semi: Token![;] = input.parse()?;
        Ok(MethodDecl { attrs, vis, sig })
    }
}

pub fn expand(mut item: ItemImpl) -> Result<TokenStream, Error> {
    if item.trait_.is_some() {
        return Err(Error::new_spanned(
            &item,
            "#[object_impl] applies to the type's inherent impl, not a trait impl",
        ));
    }
    if !item.generics.params.is_empty() {
        return Err(Error::new_spanned(
            &item.generics,
            "#[object_impl] does not support generic impls",
        ));
    }
    normalize_bodyless_methods(&mut item.items)?;

    let mut signals = Vec::new();
    let mut slots = Vec::new();
    for impl_item in &mut item.items {
        let ImplItem::Fn(method) = impl_item else {
            continue;
        };
        match take_member_attr(method)? {
            Some(Member::Signal) => signals.push(signal_descriptor(method)?),
            Some(Member::Slot) => slots.push(slot_descriptor(method, &item.self_ty)?),
            None => {}
        }
    }

    let self_ty = &item.self_ty;
    let signal_descriptors = signals.iter().map(|s| s.descriptor.clone());
    let slot_descriptors = slots.iter().map(|(descriptor, _)| descriptor.clone());
    let mut consts = TokenStream::new();
    for (index, signal) in signals.iter().enumerate() {
        let ident = &signal.const_ident;
        let args_ty = &signal.args_ty;
        let doc = format!(
            "Signal token; use `Hub::emit(handle, Self::{ident}, args)`."
        );
        let index = index as u32;
        consts.extend(quote! {
            #[doc = #doc]
            pub const #ident: ::rt_core::signal::SignalId<#self_ty, #args_ty> =
                ::rt_core::signal::SignalId::new(#index);
        });
    }
    for (_, adapter) in &slots {
        consts.extend(adapter.clone());
    }
    Ok(quote! {
        #item

        #[automatically_derived]
        impl ::rt_core::meta::ObjectInputs for #self_ty {
            const SIGNALS: &'static [::rt_core::meta::SignalDescriptor] = &[#(#signal_descriptors),*];
            const SLOTS: &'static [::rt_core::meta::SlotDescriptor] = &[#(#slot_descriptors),*];
        }

        impl #self_ty {
            #consts
        }
    })
}

/// Re-parses verbatim bodyless methods into `ImplItem::Fn` with empty blocks,
/// so signal harvesting and body generation see them.
fn normalize_bodyless_methods(items: &mut [ImplItem]) -> Result<(), Error> {
    for impl_item in items.iter_mut() {
        if let ImplItem::Verbatim(tokens) = impl_item
            && let Ok(decl) = syn::parse2::<MethodDecl>(tokens.clone())
        {
            if decl.attrs.iter().any(|attr| attr.path().is_ident("slot")) {
                return Err(Error::new_spanned(
                    tokens,
                    "#[slot] methods need a body; bodyless declarations are #[signal]s",
                ));
            }
            *impl_item = ImplItem::Fn(ImplItemFn {
                attrs: decl.attrs,
                vis: decl.vis,
                defaultness: None,
                sig: decl.sig,
                block: syn::parse_quote!({}),
            });
        }
    }
    Ok(())
}

/// Takes (strips) `#[signal]`/`#[slot]`, rejecting a method marked as both.
fn take_member_attr(method: &mut ImplItemFn) -> Result<Option<Member>, Error> {
    let mut member = None;
    let mut kept = Vec::new();
    for attr in method.attrs.drain(..) {
        if attr.path().is_ident("signal") || attr.path().is_ident("slot") {
            let found = if attr.path().is_ident("signal") {
                Member::Signal
            } else {
                Member::Slot
            };
            if member.replace(found).is_some() {
                return Err(Error::new_spanned(
                    &attr,
                    "a method is either #[signal] or #[slot], not both",
                ));
            }
        } else {
            kept.push(attr);
        }
    }
    method.attrs = kept;
    Ok(member)
}

/// Shared signature rules: plain, non-generic, referenced receiver, plain ident args.
/// Returns the arg type descriptors.
fn validate_signature(sig: &Signature, what: &str) -> Result<Vec<TokenStream>, Error> {
    if !sig.generics.params.is_empty() {
        return Err(Error::new_spanned(
            &sig.generics,
            format!("{what} cannot be generic"),
        ));
    }
    if sig.constness.is_some()
        || sig.asyncness.is_some()
        || sig.unsafety.is_some()
        || sig.abi.is_some()
        || sig.variadic.is_some()
    {
        return Err(Error::new_spanned(
            sig,
            format!("{what} must be plain methods"),
        ));
    }
    let Some(receiver) = sig.receiver() else {
        return Err(Error::new_spanned(
            sig,
            format!("{what} must be methods taking self"),
        ));
    };
    if receiver.reference.is_none() {
        return Err(Error::new_spanned(
            receiver,
            format!("{what} take &self or &mut self"),
        ));
    }
    let mut args = Vec::new();
    for input in &sig.inputs {
        let FnArg::Typed(arg) = input else { continue };
        if !matches!(arg.pat.as_ref(), Pat::Ident(_) | Pat::Wild(_)) {
            return Err(Error::new_spanned(
                &arg.pat,
                format!("{what} arguments must be plain identifiers"),
            ));
        }
        args.push(type_ref(&arg.ty));
    }
    Ok(args)
}

fn signal_descriptor(method: &mut ImplItemFn) -> Result<SignalInfo, Error> {
    let sig = &method.sig;
    if sig
        .receiver()
        .is_some_and(|r: &Receiver| r.mutability.is_some())
    {
        return Err(Error::new_spanned(
            sig.receiver(),
            "signals take &self, not &mut self",
        ));
    }
    if !matches!(sig.output, ReturnType::Default) {
        return Err(Error::new_spanned(
            &sig.output,
            "signals return nothing; drop the return type",
        ));
    }
    let args = validate_signature(sig, "signals")?;
    let name = sig.ident.to_string();
    let const_ident = Ident::new(&name.to_uppercase(), sig.ident.span());
    let arg_tys: Vec<_> = sig
        .inputs
        .iter()
        .filter_map(|input| match input {
            FnArg::Typed(arg) => Some((*arg.ty).clone()),
            _ => None,
        })
        .collect();
    let args_ty = match arg_tys.len() {
        0 => quote!(()),
        1 => quote!(#(#arg_tys)*),
        _ => quote!((#(#arg_tys),*)),
    };
    method.block = syn::parse_quote!({
        ::core::panic!("rt: emit through Hub::emit(handle, Self::#const_ident, args)")
    });
    method
        .attrs
        .push(syn::parse_quote!(#[allow(unused_variables)]));
    Ok(SignalInfo {
        descriptor: quote! {
            ::rt_core::meta::SignalDescriptor {
                name: #name,
                args: &[#(#args),*],
            }
        },
        const_ident,
        args_ty,
    })
}

fn slot_descriptor(
    method: &ImplItemFn,
    self_ty: &syn::Type,
) -> Result<(TokenStream, TokenStream), Error> {
    let sig = &method.sig;
    let args = validate_signature(sig, "slots")?;
    let ret = match &sig.output {
        ReturnType::Default => quote!(::rt_core::meta::TypeRef {
            name: "()",
            id: ::core::any::TypeId::of::<()>(),
        }),
        ReturnType::Type(_, ty) => type_ref(ty),
    };
    let name = sig.ident.to_string();
    let ident = &sig.ident;
    let arg_tys: Vec<_> = sig
        .inputs
        .iter()
        .filter_map(|input| match input {
            FnArg::Typed(arg) => Some((*arg.ty).clone()),
            _ => None,
        })
        .collect();
    // Adapters match a signal's payload arity (unit / tuple) so every slot
    // fits `fn(&mut Recv, Args)`; one-argument methods are used as-is.
    let adapter_name = Ident::new(&format!("{ident}_slot"), ident.span());
    let adapter = match arg_tys.len() {
        0 => quote! {
            #[doc = "Slot adapter: takes the unit signal payload."]
            pub fn #adapter_name(recv: &mut #self_ty, _args: ()) {
                recv.#ident();
            }
        },
        1 => quote! {},
        _ => {
            let indices: Vec<_> = (0..arg_tys.len()).map(syn::Index::from).collect();
            quote! {
                #[doc = "Slot adapter: takes the signal payload as a tuple."]
                pub fn #adapter_name(recv: &mut #self_ty, args: (#(#arg_tys),*)) {
                    recv.#ident(#(args.#indices),*);
                }
            }
        }
    };
    let descriptor = quote! {
        ::rt_core::meta::SlotDescriptor {
            name: #name,
            args: &[#(#args),*],
            ret: #ret,
        }
    };
    Ok((descriptor, adapter))
}
