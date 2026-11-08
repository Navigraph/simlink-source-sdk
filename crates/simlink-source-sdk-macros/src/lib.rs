//! `protocol!`, which turns Simlink's source manifest into the SDK's protocol API.

mod protocol;

use proc_macro::TokenStream;
use syn::Error;

/// Expands a Simlink source manifest, relative to the crate root, into exported Rust items.
#[proc_macro]
pub fn protocol(input: TokenStream) -> TokenStream {
    let path = syn::parse_macro_input!(input as syn::LitStr);
    protocol::expand(&path)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}
