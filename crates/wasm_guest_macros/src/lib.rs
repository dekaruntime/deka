extern crate proc_macro;

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse::Parse, parse_macro_input, ItemFn, LitStr};

#[proc_macro_attribute]
pub fn export_json(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);
    let sig = &input.sig;
    let name = sig.ident.to_string();

    let expanded = quote! {
        #input

        #[doc(hidden)]
        pub fn __deka_exported_name() -> &'static str { #name }
    };

    expanded.into()
}

/// Declare the JSON dispatcher entry point for the listed functions.
///
/// Example:
/// deka_export_json!(greet, add);
#[proc_macro]
pub fn deka_export_json(input: TokenStream) -> TokenStream {
    struct IdentList {
        items: syn::punctuated::Punctuated<syn::Ident, syn::Token![,]>,
    }
    impl Parse for IdentList {
        fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
            let items =
                syn::punctuated::Punctuated::<syn::Ident, syn::Token![,]>::parse_terminated(input)?;
            Ok(Self { items })
        }
    }

    let list = parse_macro_input!(input as IdentList)
        .items
        .into_iter()
        .collect::<Vec<_>>();

    let names: Vec<LitStr> = list
        .iter()
        .map(|ident| LitStr::new(&ident.to_string(), ident.span()))
        .collect();

    let expanded = quote! {
        #[no_mangle]
        pub extern "C" fn deka_call(
            name_ptr: *const u8,
            name_len: u32,
            args_ptr: *const u8,
            args_len: u32,
        ) -> *mut deka_wasm_guest::WasmResult {
            let name = deka_wasm_guest::read_string(name_ptr, name_len);
            let args_json = deka_wasm_guest::read_string(args_ptr, args_len);
            let parsed: ::serde_json::Value = ::serde_json::from_str(&args_json)
                .unwrap_or_else(|_| ::serde_json::Value::Array(Vec::new()));
            let args = match parsed {
                ::serde_json::Value::Array(items) => items,
                _ => Vec::new(),
            };

            let result = match name.as_str() {
                #( #names => {
                    let value = #list(args);
                    value
                } )*
                _ => ::serde_json::Value::String("unknown export".to_string()),
            };

            let json = ::serde_json::to_string(&result).unwrap_or_else(|_| "null".to_string());
            deka_wasm_guest::write_result(&json)
        }
    };

    expanded.into()
}
