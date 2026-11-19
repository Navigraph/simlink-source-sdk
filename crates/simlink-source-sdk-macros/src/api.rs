//! The `Event` enum and `emit_*` methods, per command a handler trait, a reply object and an `on_*` method, and the
//! `Channel` enum rejections name.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use crate::manifest::Manifest;
use crate::types::Types;
use heck::{ToSnakeCase, ToUpperCamelCase};

pub fn generate(manifest: &Manifest, types: &Types) -> TokenStream {
    let (variants, emits): (Vec<_>, Vec<_>) = manifest
        .events
        .iter()
        .zip(&types.payloads)
        .map(|((name, event), payload)| {
            let doc = &event.description;
            let variant = format_ident!("{}", name.to_upper_camel_case());
            let method = format_ident!("emit_{}", name.to_snake_case());
            let rate = event
                .max_rate_hz
                .map(|rate| format!("Sent at most {rate} Hz; the latest payload wins."))
                .unwrap_or_default();
            let method_doc = sentences(&[doc, &rate]);
            let deprecated = deprecated(&event.deprecation);
            (
                quote!(#[doc = #doc] #deprecated #variant),
                quote! {
                    #[doc = #method_doc]
                    #deprecated
                    pub fn #method(&self, payload: #payload) -> ::std::result::Result<(), crate::Error> {
                        self.emit_json(Event::#variant, &payload)
                    }
                },
            )
        })
        .unzip();

    let commands = manifest.commands.iter().zip(&types.commands).map(
        |((name, command), [input, output, refusal])| {
            let doc = &command.description;
            let base = name.to_upper_camel_case();
            let (handler, reply) = (format_ident!("{base}Handler"), format_ident!("{base}Reply"));
            let method = format_ident!("on_{}", name.to_snake_case());
            let timeout = format!(
                "Answer within {} ms. Register before `start`.",
                command.timeout_ms
            );
            let method_doc = sentences(&[doc, &timeout]);
            let handler_doc =
                format!("Answers `{name}` through `reply`, now or later from any thread.");
            let reply_doc = format!(
                "Answers one `{name}` command. Dropping it unanswered lets the command time out."
            );
            let deprecated = deprecated(&command.deprecation);
            quote! {
                #[doc = #handler_doc]
                #[::uniffi::export(with_foreign)]
                pub trait #handler: Send + Sync {
                    /// Called on the SDK's thread, so keep it short.
                    fn handle(&self, input: #input, reply: ::std::sync::Arc<#reply>);
                }

                #[doc = #reply_doc]
                #[derive(::uniffi::Object)]
                pub struct #reply(crate::PendingReply);

                #[::uniffi::export]
                impl #reply {
                    /// Answers with the command's output.
                    pub fn ok(&self, value: #output) -> ::std::result::Result<(), crate::Error> {
                        self.0.ok(&value)
                    }

                    /// Refuses with one of the command's declared error codes.
                    pub fn refuse(&self, error: #refusal) -> ::std::result::Result<(), crate::Error> {
                        self.0.refuse(&error)
                    }
                }

                #[::uniffi::export]
                impl Source {
                    #[doc = #method_doc]
                    #deprecated
                    pub fn #method(
                        &self,
                        handler: ::std::sync::Arc<dyn #handler>,
                    ) -> ::std::result::Result<(), crate::Error> {
                        self.handle(#name, move |input, reply| {
                            handler.handle(input, ::std::sync::Arc::new(#reply(reply)))
                        })
                    }
                }
            }
        },
    );

    let channel = channel(manifest);

    quote! {
        use crate::Source;

        /// Event channels a source can declare.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, ::uniffi::Enum)]
        #[non_exhaustive]
        pub enum Event {
            #( #variants, )*
        }

        impl Event {
            pub(crate) fn info(self) -> &'static crate::EventInfo {
                // `EVENTS` lists the variants in order.
                &EVENTS[self as usize]
            }
        }

        // The SDK keeps serving deprecated channels.
        #[allow(deprecated)]
        #[::uniffi::export]
        impl Source {
            #(#emits)*
        }

        #(#commands)*

        #channel
    }
}

/// Every event and command, as Simlink names them in a rejection, with an `Other` variant for what a newer Simlink
/// sends.
fn channel(manifest: &Manifest) -> TokenStream {
    let (variants, reads, names): (Vec<_>, Vec<_>, Vec<_>) = manifest
        .channels()
        .map(|(name, doc)| {
            let variant = format_ident!("{}", name.to_upper_camel_case());
            (
                quote!(#[doc = #doc] #variant),
                quote!(#name => Self::#variant),
                quote!(Self::#variant => #name),
            )
        })
        .collect();

    quote! {
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

fn deprecated(reason: &Option<String>) -> Option<TokenStream> {
    match reason.as_deref()? {
        "" => Some(quote!(#[deprecated])),
        reason => Some(quote!(#[deprecated = #reason])),
    }
}

/// Joins the non-empty parts as sentences, so a channel without a description still reads well.
fn sentences(parts: &[&str]) -> String {
    parts
        .iter()
        .map(|part| part.trim().trim_end_matches('.'))
        .filter(|part| !part.is_empty())
        .map(|part| format!("{part}."))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_sentences_without_empty_parts() {
        assert_eq!(
            sentences(&["Position.", "", " Sent at 10 Hz "]),
            "Position. Sent at 10 Hz."
        );
        assert_eq!(sentences(&["", ""]), "");
    }
}
