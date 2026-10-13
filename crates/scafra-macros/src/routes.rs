use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::{format_ident, quote};
use syn::{
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
    Attribute, Error, FnArg, ItemImpl, LitStr, Result, Token, Type,
};

pub(crate) fn expand_routes(
    attr: TokenStream,
    item: TokenStream,
) -> Result<proc_macro2::TokenStream> {
    let default_routes = if attr.is_empty() {
        false
    } else {
        let mode = syn::parse::<syn::Ident>(attr)?;
        if mode != "default" {
            return Err(Error::new_spanned(
                mode,
                "routes only accepts the `default` mode",
            ));
        }
        true
    };
    let mut input = syn::parse::<ItemImpl>(item)?;
    if input.trait_.is_some() {
        return Err(Error::new_spanned(
            &input.self_ty,
            "routes must be applied to an inherent impl block",
        ));
    }
    if !input.generics.params.is_empty() {
        return Err(Error::new_spanned(
            &input.generics,
            "routes currently requires a non-generic impl block",
        ));
    }

    let controller = match input.self_ty.as_ref() {
        Type::Path(path) => path
            .path
            .segments
            .last()
            .map(|segment| segment.ident.clone())
            .ok_or_else(|| {
                Error::new_spanned(&input.self_ty, "could not determine controller type")
            })?,
        _ => {
            return Err(Error::new_spanned(
                &input.self_ty,
                "routes requires a named controller type",
            ))
        }
    };

    let mut handlers = Vec::new();
    let mut registrations = Vec::new();
    let mut metadata = Vec::new();
    let mut authorization_metadata = Vec::new();
    let mut route_index = 0usize;

    for impl_item in &mut input.items {
        let syn::ImplItem::Fn(method) = impl_item else {
            continue;
        };
        let route_attributes = method
            .attrs
            .iter()
            .filter_map(parse_route_attribute)
            .collect::<Result<Vec<_>>>()?;
        let route_policy = parse_route_policy(&method.attrs)?;
        let has_policy_attributes = method.attrs.iter().any(is_policy_attribute);
        method.attrs.retain(|attribute| {
            parse_route_attribute(attribute).is_none() && !is_policy_attribute(attribute)
        });

        if route_attributes.is_empty() {
            if has_policy_attributes {
                return Err(Error::new_spanned(
                    method,
                    "authorization attributes must be attached to a route handler",
                ));
            }
            continue;
        }
        validate_handler_signature(method)?;

        for (http_method, path) in route_attributes {
            let handler = format_ident!("__scafra_route_handler_{route_index}");
            route_index += 1;
            let method_ident = &method.sig.ident;
            let arguments = method
                .sig
                .inputs
                .iter()
                .skip(1)
                .map(|argument| match argument {
                    FnArg::Typed(argument) => match argument.pat.as_ref() {
                        syn::Pat::Ident(pattern) => {
                            Ok((pattern.ident.clone(), argument.ty.clone()))
                        }
                        _ => Err(Error::new_spanned(
                            &argument.pat,
                            "route extractor arguments must use a simple identifier pattern",
                        )),
                    },
                    FnArg::Receiver(_) => Err(Error::new_spanned(
                        argument,
                        "only '&self' may be used as the controller receiver",
                    )),
                })
                .collect::<Result<Vec<_>>>()?;
            let argument_names = arguments.iter().map(|(ident, _)| ident);
            let argument_definitions = arguments.iter().map(|(ident, ty)| quote!(#ident: #ty));
            let routing_function = format_ident!("{}", http_method);
            let path_literal = LitStr::new(&path, Span::call_site());
            let http_method_literal = LitStr::new(&http_method.to_uppercase(), Span::call_site());
            let output = &method.sig.output;
            let route_policy = route_policy.to_tokens();

            handlers.push(quote! {
                async fn #handler(
                    extension: ::scafra::web::axum::extract::Extension<
                        ::std::sync::Arc<Self>
                    >,
                    #(#argument_definitions),*
                ) #output {
                    extension.0.#method_ident(#(#argument_names),*).await
                }
            });
            registrations.push(quote! {
                let path = ::scafra::web::join_paths(
                    <Self as ::scafra::web::ControllerPrefix>::PREFIX,
                    #path_literal,
                );
                router = router.route(
                    &path,
                    ::scafra::web::axum::routing::#routing_function(Self::#handler)
                        .layer(::scafra::web::axum::Extension(controller.clone())),
                );
            });
            metadata.push(quote! {
                ::scafra::web::RouteMetadata {
                    controller: stringify!(#controller),
                    method: #http_method_literal,
                    prefix: <#controller as ::scafra::web::ControllerPrefix>::PREFIX,
                    path: #path_literal,
                }
            });
            authorization_metadata.push(quote! {
                ::scafra::web::RouteAuthorizationMetadata {
                    controller: stringify!(#controller),
                    method: #http_method_literal,
                    prefix: <#controller as ::scafra::web::ControllerPrefix>::PREFIX,
                    path: #path_literal,
                    controller_policy: <#controller as ::scafra::web::ControllerPrefix>::AUTHORIZATION_POLICY,
                    route_policy: #route_policy,
                }
            });
        }
    }

    let metadata_ident = format_ident!("__SCAFRA_{}_ROUTES", controller);
    let authorization_metadata_ident = format_ident!("__SCAFRA_{}_AUTHORIZATION", controller);
    let register_ident = format_ident!("__scafra_register_default_{}", controller);
    let default_registration = if default_routes {
        quote!(<#controller as ::scafra::web::ControllerRoutes>::register_routes(router))
    } else {
        quote!(router)
    };
    Ok(quote! {
        #input

        impl #controller {
            #(#handlers)*

            #[doc(hidden)]
            pub fn __scafra_register_routes_with(
                mut router: ::scafra::web::axum::Router,
                controller: Self,
            ) -> ::scafra::web::axum::Router {
                let controller = ::std::sync::Arc::new(controller);
                #(#registrations)*
                router
            }
        }

        impl ::scafra::web::ControllerRoutes for #controller {
            fn register_routes_with(
                router: ::scafra::web::axum::Router,
                controller: Self,
            ) -> ::scafra::web::axum::Router {
                Self::__scafra_register_routes_with(router, controller)
            }

            fn route_metadata() -> &'static [::scafra::web::RouteMetadata] {
                #metadata_ident
            }
        }

        #[doc(hidden)]
        #[allow(non_upper_case_globals)]
        const #metadata_ident: &[::scafra::web::RouteMetadata] = &[
            #(#metadata),*
        ];

        #[doc(hidden)]
        #[allow(non_upper_case_globals)]
        const #authorization_metadata_ident: &[::scafra::web::RouteAuthorizationMetadata] = &[
            #(#authorization_metadata),*
        ];

        #[doc(hidden)]
        #[allow(non_snake_case)]
        fn #register_ident(router: ::scafra::web::axum::Router) -> ::scafra::web::axum::Router {
            // Graph-composed controllers are registered with their constructed
            // instances. Legacy default construction is opt-in with
            // `#[routes(default)]`.
            #default_registration
        }

        ::scafra::web::__private::inventory::submit! {
            ::scafra::web::ControllerRegistration {
                controller: stringify!(#controller),
                register: #register_ident,
                routes: #metadata_ident,
            }
        }

        ::scafra::web::__private::inventory::submit! {
            ::scafra::web::ControllerAuthorizationRegistration {
                controller: stringify!(#controller),
                routes: #authorization_metadata_ident,
            }
        }
    })
}

fn is_policy_attribute(attribute: &Attribute) -> bool {
    attribute.path().is_ident("public")
        || attribute.path().is_ident("authenticated")
        || attribute.path().is_ident("roles")
        || attribute.path().is_ident("scopes")
}

#[derive(Default)]
struct RoutePolicyArgs {
    explicit_mode: Option<RoutePolicyMode>,
    role_groups: Vec<Vec<LitStr>>,
    required_scopes: Vec<LitStr>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RoutePolicyMode {
    Public,
    Protected,
}

impl Parse for RoleNames {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let names = Punctuated::<LitStr, Token![,]>::parse_terminated(input)?;
        if names.is_empty() {
            return Err(Error::new(input.span(), "at least one name is required"));
        }
        for name in &names {
            if name.value().trim().is_empty()
                || name.value().bytes().any(|byte| byte.is_ascii_whitespace())
            {
                return Err(Error::new_spanned(
                    name,
                    "role and scope names must be non-empty and contain no whitespace",
                ));
            }
        }
        Ok(Self(names.into_iter().collect()))
    }
}

struct RoleNames(Vec<LitStr>);

fn parse_route_policy(attributes: &[Attribute]) -> Result<RoutePolicyArgs> {
    let mut policy = RoutePolicyArgs::default();
    for attribute in attributes
        .iter()
        .filter(|attribute| is_policy_attribute(attribute))
    {
        if attribute.path().is_ident("public") {
            if !matches!(attribute.meta, syn::Meta::Path(_)) {
                return Err(Error::new_spanned(attribute, "`public` takes no arguments"));
            }
            policy.set_mode(RoutePolicyMode::Public, attribute)?;
        } else if attribute.path().is_ident("authenticated") {
            if !matches!(attribute.meta, syn::Meta::Path(_)) {
                return Err(Error::new_spanned(
                    attribute,
                    "`authenticated` takes no arguments",
                ));
            }
            policy.set_mode(RoutePolicyMode::Protected, attribute)?;
        } else if attribute.path().is_ident("roles") {
            policy
                .role_groups
                .push(attribute.parse_args::<RoleNames>()?.0);
        } else if attribute.path().is_ident("scopes") {
            policy
                .required_scopes
                .extend(attribute.parse_args::<RoleNames>()?.0);
        }
    }
    policy.finish()?;
    Ok(policy)
}

impl RoutePolicyArgs {
    fn set_mode(&mut self, mode: RoutePolicyMode, attribute: &Attribute) -> Result<()> {
        if self.explicit_mode.replace(mode).is_some() {
            return Err(Error::new_spanned(
                attribute,
                "route policy may declare `public` or `authenticated` only once",
            ));
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        let has_requirements = !self.role_groups.is_empty() || !self.required_scopes.is_empty();
        if self.explicit_mode == Some(RoutePolicyMode::Public) && has_requirements {
            return Err(Error::new(
                proc_macro2::Span::call_site(),
                "a public route cannot also require roles or scopes",
            ));
        }
        if has_requirements && self.explicit_mode.is_none() {
            self.explicit_mode = Some(RoutePolicyMode::Protected);
        }
        Ok(())
    }

    fn to_tokens(&self) -> proc_macro2::TokenStream {
        let mode = match self.explicit_mode {
            None => quote!(::scafra::web::AuthorizationMode::Inherit),
            Some(RoutePolicyMode::Public) => quote!(::scafra::web::AuthorizationMode::Public),
            Some(RoutePolicyMode::Protected) => {
                quote!(::scafra::web::AuthorizationMode::Protected)
            }
        };
        let role_groups = self.role_groups.iter().map(|group| quote!(&[#(#group),*]));
        let scopes = &self.required_scopes;
        quote! {
            ::scafra::web::AuthorizationPolicy {
                mode: #mode,
                role_groups: &[#(#role_groups),*],
                required_scopes: &[#(#scopes),*],
            }
        }
    }
}

fn parse_route_attribute(attribute: &Attribute) -> Option<Result<(String, String)>> {
    let method = if attribute.path().is_ident("get") {
        "get"
    } else if attribute.path().is_ident("post") {
        "post"
    } else if attribute.path().is_ident("put") {
        "put"
    } else if attribute.path().is_ident("delete") {
        "delete"
    } else {
        return None;
    };
    Some(
        attribute
            .parse_args::<LitStr>()
            .map(|path| (method.to_owned(), path.value())),
    )
}

fn validate_handler_signature(method: &syn::ImplItemFn) -> Result<()> {
    if method.sig.asyncness.is_none() {
        return Err(Error::new_spanned(
            method.sig.fn_token,
            "route handlers must be async functions",
        ));
    }
    match method.sig.inputs.first() {
        Some(FnArg::Receiver(receiver))
            if receiver.reference.is_some() && receiver.mutability.is_none() => {}
        Some(argument) => {
            return Err(Error::new_spanned(
                argument,
                "route handlers must use '&self' as their first argument",
            ))
        }
        None => {
            return Err(Error::new_spanned(
                &method.sig.ident,
                "route handlers must have a '&self' receiver",
            ))
        }
    }
    Ok(())
}
