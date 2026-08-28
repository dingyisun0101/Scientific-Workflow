//! Declaration macros for `scientific-workflow`.

use proc_macro::TokenStream;
use quote::quote;
use syn::{ItemImpl, LitStr, parse_macro_input};

/// Registers one `ExecutionUnit` implementation under a stable manifest key.
#[proc_macro_attribute]
pub fn model(attribute: TokenStream, item: TokenStream) -> TokenStream {
    expand_registration(attribute, item)
}

/// Registers one `ExecutionUnit` implementation under a stable manifest key.
#[proc_macro_attribute]
pub fn execution_unit(attribute: TokenStream, item: TokenStream) -> TokenStream {
    expand_registration(attribute, item)
}

fn expand_registration(attribute: TokenStream, item: TokenStream) -> TokenStream {
    let key = parse_macro_input!(attribute as LitStr);
    let implementation = parse_macro_input!(item as ItemImpl);
    let model_type = &implementation.self_ty;

    quote! {
        #implementation

        ::scientific_workflow::__private::inventory::submit! {
            ::scientific_workflow::__private::ModelRegistration::new::<#model_type>(#key)
        }
    }
    .into()
}
