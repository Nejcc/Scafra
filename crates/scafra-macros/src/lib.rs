//! Procedural macros for Scafra.
//!
//! The macros in this crate intentionally generate small, inspectable impls.
//! They do not perform runtime type discovery.

use proc_macro::TokenStream;

mod bean;
mod config;
mod controller;
mod diagnostics;
mod logger;
#[path = "main.rs"]
mod main_macro;
mod parse;
mod routes;
mod service;

#[proc_macro_attribute]
pub fn service(attr: TokenStream, item: TokenStream) -> TokenStream {
    diagnostics::into_token_stream(service::expand_service(attr, item))
}

#[proc_macro_attribute]
pub fn component(attr: TokenStream, item: TokenStream) -> TokenStream {
    diagnostics::into_token_stream(service::expand_service(attr, item))
}

#[proc_macro_attribute]
pub fn repository(attr: TokenStream, item: TokenStream) -> TokenStream {
    diagnostics::into_token_stream(service::expand_service(attr, item))
}

#[proc_macro_attribute]
pub fn bean(attr: TokenStream, item: TokenStream) -> TokenStream {
    diagnostics::into_token_stream(bean::expand_bean(attr, item))
}

#[proc_macro_attribute]
pub fn post_processor(attr: TokenStream, item: TokenStream) -> TokenStream {
    diagnostics::into_token_stream(bean::expand_post_processor(attr, item))
}

#[proc_macro_derive(Config, attributes(config))]
pub fn config(input: TokenStream) -> TokenStream {
    diagnostics::into_token_stream(config::expand_config(input))
}

#[proc_macro_attribute]
pub fn controller(attr: TokenStream, item: TokenStream) -> TokenStream {
    diagnostics::into_token_stream(controller::expand_controller(attr, item))
}

#[proc_macro_attribute]
pub fn routes(_attr: TokenStream, item: TokenStream) -> TokenStream {
    diagnostics::into_token_stream(routes::expand_routes(_attr, item))
}

#[proc_macro_attribute]
pub fn main(_attr: TokenStream, item: TokenStream) -> TokenStream {
    diagnostics::into_token_stream(main_macro::expand_main(item.into()))
}

/// Adds structured logging spans to functions and impl methods.
#[proc_macro_attribute]
pub fn logger(_attr: TokenStream, item: TokenStream) -> TokenStream {
    diagnostics::into_token_stream(logger::expand_logger(item.into()))
}

// These are standalone no-op attributes. The routes macro consumes them when
// attached to an impl; keeping the exports gives normal Rust behavior when a
// route attribute is used outside that macro.
#[proc_macro_attribute]
pub fn get(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

#[proc_macro_attribute]
pub fn post(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

#[proc_macro_attribute]
pub fn put(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

#[proc_macro_attribute]
pub fn delete(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

#[cfg(test)]
mod tests {
    use super::main_macro::expand_main_item;
    use syn::ItemFn;

    #[test]
    fn main_expansion_uses_bounded_startup_logging() {
        let input: ItemFn = syn::parse_quote! {
            async fn main() {}
        };
        let expanded = expand_main_item(input)
            .expect("valid main should expand")
            .to_string();

        assert!(expanded.contains("log_startup_failure"));
        assert!(expanded.contains("run_with_resources"));
        assert!(expanded.contains("__scafra_graph :: compose_async"));
        assert!(!expanded.contains("% error"));
        assert!(!expanded.contains("Scafra application failed"));
    }

    #[test]
    fn main_expansion_keeps_graph_resources_until_shutdown() {
        let input: ItemFn = syn::parse_quote! {
            async fn main() {}
        };
        let expanded = expand_main_item(input)
            .expect("valid main should expand")
            .to_string();
        assert!(expanded.contains("build_router_and_resources"));
        assert!(expanded.contains("AsyncResourceSet :: shutdown"));
        assert!(expanded.contains("StartupFailure"));
    }
}
