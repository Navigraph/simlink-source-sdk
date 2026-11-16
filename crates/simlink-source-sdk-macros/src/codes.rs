//! `HandshakeError`, `RejectionCode` and `Channel`: what Simlink refuses and rejects with, each with an `Other` variant
//! for what a newer Simlink sends.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use crate::manifest::{Code, FieldType, Manifest};
use heck::{ToSnakeCase, ToUpperCamelCase};

pub fn generate(manifest: &Manifest) -> TokenStream {
    let handshake_error = code_enum(
        "HandshakeError",
        "Why Simlink refused the handshake.",
        &manifest.handshake_errors,
    );
    let rejection_code = code_enum(
        "RejectionCode",
        "Why Simlink rejected a payload or answer.",
        &manifest.rejection_codes,
    );

    let (variants, reads, names): (Vec<_>, Vec<_>, Vec<_>) = manifest
        .channels()
        .map(|channel| {
            let (name, doc) = (&channel.name, &channel.doc);
            let variant = format_ident!("{}", name.to_upper_camel_case());
            (
                quote!(#[doc = #doc] #variant),
                quote!(#name => Self::#variant),
                quote!(Self::#variant => #name),
            )
        })
        .collect();

    quote! {
        #handshake_error
        #rejection_code

        /// An event or command, as Simlink names it in a rejection.
        #[derive(Clone, Debug, PartialEq, Eq, ::uniffi::Enum)]
        #[non_exhaustive]
        pub enum Channel {
            #( #variants, )*
            /// A channel this SDK doesn't know, from a newer Simlink.
            Other { name: String },
        }

        impl Channel {
            pub(crate) fn read(name: &str) -> Self {
                match name {
                    #( #reads, )*
                    name => Self::Other { name: name.to_owned() },
                }
            }
        }

        #[::uniffi::export]
        impl Channel {
            /// The name as Simlink sends it, such as `telemetry.ownship`.
            pub fn wire_name(&self) -> String {
                match self {
                    #( #names, )*
                    Self::Other { name } => name,
                }
                .to_owned()
            }
        }
    }
}

/// An enum of `codes` read from a code and its metadata.
fn code_enum(name: &str, doc: &str, codes: &[Code]) -> TokenStream {
    let name = format_ident!("{name}");
    let (variants, reads, literals): (Vec<_>, Vec<_>, Vec<_>) = codes
        .iter()
        .map(|code| {
            let (literal, doc) = (&code.code, &code.doc);
            let variant = format_ident!("{}", code.variant);
            let fields = code.fields.iter().map(|field| {
                let name = format_ident!("{}", field.key.to_snake_case());
                let ty = rust_type(&field.ty);
                let ty = if field.required {
                    ty
                } else {
                    quote!(Option<#ty>)
                };
                quote!(#name: #ty)
            });
            let reads = code.fields.iter().map(|field| {
                let (name, key) = (format_ident!("{}", field.key.to_snake_case()), &field.key);
                quote!(#name: field(metadata, #key))
            });
            let (declare, read) = if code.fields.is_empty() {
                (quote!(#variant), quote!(Self::#variant))
            } else {
                (
                    quote!(#variant { #(#fields),* }),
                    quote!(Self::#variant { #(#reads),* }),
                )
            };
            (
                quote!(#[doc = #doc] #declare),
                quote!(#literal => #read),
                quote!(Self::#variant { .. } => #literal),
            )
        })
        .collect();

    quote! {
        #[doc = #doc]
        #[derive(Clone, Debug, PartialEq, ::uniffi::Enum)]
        #[non_exhaustive]
        pub enum #name {
            #( #variants, )*
            /// A code this SDK doesn't know, from a newer Simlink.
            Other { code: String },
        }

        impl #name {
            /// Reads `code`. Metadata Simlink left out or malformed reads as empty.
            pub(crate) fn read(code: &str, metadata: &::serde_json::Value) -> Self {
                fn field<T: ::serde::de::DeserializeOwned + Default>(
                    metadata: &::serde_json::Value,
                    key: &str,
                ) -> T {
                    metadata
                        .get(key)
                        .and_then(|value| ::serde_json::from_value(value.clone()).ok())
                        .unwrap_or_default()
                }

                match code {
                    #( #reads, )*
                    code => Self::Other { code: code.to_owned() },
                }
            }
        }

        #[::uniffi::export]
        impl #name {
            /// The code as Simlink sends it.
            pub fn wire_code(&self) -> String {
                match self {
                    #( #literals, )*
                    Self::Other { code } => code,
                }
                .to_owned()
            }
        }
    }
}

fn rust_type(ty: &FieldType) -> TokenStream {
    match ty {
        FieldType::String => quote!(String),
        FieldType::Integer => quote!(i64),
        FieldType::Number => quote!(f64),
        FieldType::Boolean => quote!(bool),
        FieldType::Array(item) => {
            let item = rust_type(item);
            quote!(Vec<#item>)
        }
    }
}
