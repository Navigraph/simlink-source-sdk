//! `protocol!("path/to/source-schema.json")`: Simlink's source manifest as exported Rust items.
//!
//! `manifest` reads the manifest into a model and fails on anything the SDK doesn't handle. The other modules each
//! generate part of the API from that model: the protocol's types via typify (`types`), the `Event` and `Channel`
//! enums and the typed `Source` methods per event and command (`api`), and the constants the SDK connects with
//! (`consts`). Every API item is a UniFFI item, so bindings never read the manifest.

mod api;
mod consts;
mod manifest;
mod types;

use quote::quote;
use syn::{Error, LitStr};

/// Expands a Simlink source manifest, relative to the crate root, into exported Rust items.
#[proc_macro]
pub fn protocol(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let path = syn::parse_macro_input!(input as LitStr);
    expand(&path)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

fn expand(path: &LitStr) -> syn::Result<proc_macro2::TokenStream> {
    let fail = |message: String| Error::new(path.span(), message);
    let dir = std::env::var("CARGO_MANIFEST_DIR").map_err(|e| fail(e.to_string()))?;
    let file = std::path::Path::new(&dir).join(path.value());
    let text = std::fs::read_to_string(&file)
        .map_err(|e| fail(format!("reading {}: {e}", file.display())))?;

    let manifest = manifest::read(&text).map_err(fail)?;
    let types = types::generate(&manifest).map_err(fail)?;
    let type_items = &types.tokens;
    let api = api::generate(&manifest, &types);
    let consts = consts::generate(&manifest);
    let file = file.display().to_string();

    Ok(quote! {
        // Rebuild when the manifest changes.
        const _: &[u8] = include_bytes!(#file);

        #type_items
        #api
        #consts
    })
}
