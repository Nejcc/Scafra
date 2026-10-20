use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use syn::{Data, DeriveInput, Error, FnArg, ItemFn, Result, ReturnType, Type};

pub(crate) fn expand_bean(
    attr: TokenStream,
    item: TokenStream,
) -> Result<proc_macro2::TokenStream> {
    if !attr.is_empty() {
        return Err(Error::new(
            Span::call_site(),
            "the bean attribute takes no arguments",
        ));
    }
    let input = syn::parse::<ItemFn>(item)?;
    let asynchronous = input.sig.asyncness.is_some();
    if !input.sig.generics.params.is_empty() {
        return Err(Error::new_spanned(
            &input.sig,
            "bean providers currently require non-generic functions",
        ));
    }
    let default_arguments = input
        .sig
        .inputs
        .iter()
        .map(|argument| match argument {
            FnArg::Typed(_) => Ok(quote!(::std::default::Default::default())),
            FnArg::Receiver(receiver) => Err(Error::new_spanned(
                receiver,
                "bean providers cannot have a self receiver",
            )),
        })
        .collect::<Result<Vec<_>>>()?;
    let return_type = match &input.sig.output {
        ReturnType::Type(_, ty) => ty,
        ReturnType::Default => {
            return Err(Error::new_spanned(
                &input.sig.output,
                "bean providers must declare a concrete return type",
            ))
        }
    };
    let (bean_output, fallible) = bean_output_type(return_type)?;
    if matches!(bean_output, Type::Reference(_) | Type::ImplTrait(_)) {
        return Err(Error::new_spanned(
            bean_output,
            "bean providers must return an owned concrete type",
        ));
    }
    let function = &input.sig.ident;
    let default_impl = if fallible || asynchronous {
        quote! {}
    } else {
        quote! {
            impl ::std::default::Default for #bean_output {
                fn default() -> Self {
                    #function(#(#default_arguments),*)
                }
            }
        }
    };
    Ok(quote! {
        #input

        #default_impl

        impl ::scafra::core::Bean for #bean_output {}

        ::scafra::core::__private::inventory::submit! {
            ::scafra::core::ComponentRegistration {
                name: stringify!(#function),
                kind: ::scafra::core::ComponentKind::Bean,
            }
        }
    })
}

fn bean_output_type(return_type: &Type) -> Result<(&Type, bool)> {
    if let Type::Path(path) = return_type {
        if let Some(segment) = path.path.segments.last() {
            if segment.ident == "Result" {
                if let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments {
                    let mut types = arguments.args.iter();
                    if let Some(syn::GenericArgument::Type(output)) = types.next() {
                        if matches!(types.next(), Some(syn::GenericArgument::Type(_)))
                            && arguments.args.len() == 2
                        {
                            return Ok((output, true));
                        }
                    }
                }
            }
        }
    }
    Ok((return_type, false))
}
pub(crate) fn expand_post_processor(
    attr: TokenStream,
    item: TokenStream,
) -> Result<proc_macro2::TokenStream> {
    if !attr.is_empty() {
        return Err(Error::new(
            Span::call_site(),
            "the post_processor attribute takes no arguments",
        ));
    }
    let input = syn::parse::<DeriveInput>(item)?;
    let ident = &input.ident;
    crate::parse::reject_generics(&input.generics)?;
    let data = match &input.data {
        Data::Struct(data) => data,
        _ => {
            return Err(Error::new_spanned(
                &input,
                "post_processor can only be applied to a struct",
            ))
        }
    };
    let default_construction = crate::parse::default_construction(data);
    let default_impl = if crate::parse::has_default_derive(&input.attrs) {
        quote! {}
    } else {
        quote! {
            impl ::std::default::Default for #ident {
                fn default() -> Self {
                    #default_construction
                }
            }
        }
    };
    Ok(quote! {
        #input
        #default_impl

        ::scafra::core::__private::inventory::submit! {
            ::scafra::core::PostProcessorRegistration {
                name: stringify!(#ident),
                create: || Box::new(#ident::default()),
            }
        }
    })
}
