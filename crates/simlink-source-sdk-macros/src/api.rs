//! The `Event` enum and `emit_*` methods, and per command a handler trait, a reply object and an `on_*` method.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use crate::manifest::{Channel, Manifest};
use crate::names::{pascal, sentences, snake};
use crate::types::Types;

pub fn generate(manifest: &Manifest, types: &Types) -> TokenStream {
    let (variants, emits): (Vec<_>, Vec<_>) = manifest
        .events
        .iter()
        .zip(&types.payloads)
        .map(|(event, payload)| {
            let Channel {
                name,
                doc,
                deprecated,
            } = &event.channel;
            let variant = format_ident!("{}", pascal(name));
            let method = format_ident!("emit_{}", snake(name));
            let rate = event
                .max_rate_hz
                .map(|rate| format!("Sent at most {rate} Hz; the latest payload wins."))
                .unwrap_or_default();
            let method_doc = sentences(&[doc, &rate]);
            let deprecated = self::deprecated(deprecated);
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
        |(command, [input, output, error])| {
            let Channel {
                name,
                doc,
                deprecated,
            } = &command.channel;
            let base = pascal(name);
            let (handler, reply) = (format_ident!("{base}Handler"), format_ident!("{base}Reply"));
            let method = format_ident!("on_{}", snake(name));
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
            let deprecated = self::deprecated(deprecated);
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
                    pub fn refuse(&self, error: #error) -> ::std::result::Result<(), crate::Error> {
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
    }
}

fn deprecated(reason: &Option<String>) -> Option<TokenStream> {
    match reason.as_deref()? {
        "" => Some(quote!(#[deprecated])),
        reason => Some(quote!(#[deprecated = #reason])),
    }
}
