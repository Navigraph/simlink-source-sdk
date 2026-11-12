//! `protocol!("path/to/source-schema.json")`: Simlink's source manifest as exported Rust items.
//!
//! Emits the protocol's types (via typify), the `Event` enum, typed `Source` methods per event
//! and command, a handler trait and reply object per command, the codes Simlink refuses and
//! rejects with, and the constants and event table the SDK connects with. Every API item is a
//! UniFFI item, so bindings never read the manifest.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use serde_json::{Map, Value};
use syn::{Error, LitStr};
use typify::{TypeSpace, TypeSpaceSettings};

pub fn expand(path: &LitStr) -> syn::Result<TokenStream> {
    let fail = |message: String| Error::new(path.span(), message);
    let dir = std::env::var("CARGO_MANIFEST_DIR").map_err(|e| fail(e.to_string()))?;
    let file = std::path::Path::new(&dir).join(path.value());
    let text = std::fs::read_to_string(&file)
        .map_err(|e| fail(format!("reading {}: {e}", file.display())))?;
    let mut manifest: Value = serde_json::from_str(&text).map_err(|e| fail(e.to_string()))?;
    simplify(&mut manifest);

    let mut protocol = Protocol::default();
    protocol.load(&mut manifest).map_err(fail)?;
    let file = file.display().to_string();

    let (types, idents) = protocol.types().map_err(fail)?;
    let api = protocol.api(&idents);
    let codes = protocol.codes();
    let tables = protocol.tables(&manifest["connection"]).map_err(fail)?;

    Ok(quote! {
        // Rebuild when the manifest changes.
        const _: &[u8] = include_bytes!(#file);

        #types
        #api
        #codes
        #tables
    })
}

/// Strips what typify would turn into validating newtypes or required defaults; Simlink validates
/// and fills these in.
fn simplify(schema: &mut Value) {
    match schema {
        Value::Object(map) => {
            if let Some(Value::Object(properties)) = map.get_mut("properties") {
                properties.retain(|_, property| {
                    !(property.get("const").is_some() && property.get("default").is_some())
                });
                let kept: Vec<String> = properties.keys().cloned().collect();
                if let Some(Value::Array(required)) = map.get_mut("required") {
                    required.retain(|name| {
                        name.as_str()
                            .is_some_and(|name| kept.iter().any(|kept| kept == name))
                    });
                }
            }
            for key in [
                "minLength",
                "maxLength",
                "pattern",
                "format",
                "default",
                "$schema",
            ] {
                map.remove(key);
            }
            for (key, value) in map.iter_mut() {
                match (key.as_str(), value) {
                    // Keys here are property and definition names, never keywords to strip.
                    ("properties" | "$defs" | "patternProperties", Value::Object(schemas)) => {
                        schemas.values_mut().for_each(simplify)
                    }
                    (_, value) => simplify(value),
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(simplify),
        _ => {}
    }
}

struct Root {
    name: Option<String>,
    schema: Value,
}

struct Channel {
    name: String,
    doc: String,
    deprecated: Option<String>,
}

struct EventChannel {
    channel: Channel,
    max_rate_hz: Option<f64>,
    payload: usize,
}

struct CommandChannel {
    channel: Channel,
    timeout_ms: u64,
    input: usize,
    output: usize,
    error: usize,
}

/// One of the codes Simlink refuses a handshake or rejects a frame with.
struct Code {
    code: String,
    doc: String,
    variant: syn::Ident,
    /// Each metadata field: its key, Rust name and type.
    fields: Vec<(String, syn::Ident, TokenStream)>,
}

#[derive(Default)]
struct Protocol {
    defs: Map<String, Value>,
    roots: Vec<Root>,
    events: Vec<EventChannel>,
    commands: Vec<CommandChannel>,
    handshake_errors: Vec<Code>,
    rejection_codes: Vec<Code>,
}

impl Protocol {
    fn root(&mut self, name: Option<String>, schema: &mut Value) -> Result<usize, String> {
        if let Some(Value::Object(defs)) = schema
            .as_object_mut()
            .and_then(|schema| schema.remove("$defs"))
        {
            for (def, definition) in defs {
                // All definitions share one namespace of Rust types.
                match self.defs.get(&def) {
                    Some(existing) if *existing != definition => {
                        return Err(format!(
                            "$defs.{def} is defined differently by two channels"
                        ));
                    }
                    _ => self.defs.insert(def, definition),
                };
            }
        }
        self.roots.push(Root {
            name,
            schema: schema.clone(),
        });
        Ok(self.roots.len() - 1)
    }

    fn load(&mut self, manifest: &mut Value) -> Result<(), String> {
        check_keys(manifest)?;
        self.load_descriptor(&mut manifest["descriptor"])?;

        let errors = manifest["handshakeErrors"]["oneOf"]
            .as_array()
            .ok_or("handshakeErrors.oneOf is missing")?;
        self.handshake_errors = errors
            .iter()
            .map(handshake_error)
            .collect::<Result<_, _>>()?;

        let code = &manifest["errorFrame"]["properties"]["code"];
        let codes = match (&code["const"], &code["enum"]) {
            (Value::String(code), _) => vec![code.clone()],
            (_, Value::Array(codes)) => codes
                .iter()
                .map(|code| code.as_str().map(str::to_owned))
                .collect::<Option<_>>()
                .ok_or("errorFrame.properties.code.enum has a code that isn't a string")?,
            _ => return Err("errorFrame.properties.code has no const or enum".into()),
        };
        self.rejection_codes = codes
            .into_iter()
            .map(|code| new_code(code, "", String::new(), Vec::new()))
            .collect::<Result<_, _>>()?;

        // The rejection listener reads these frame fields by name.
        for key in [
            "event",
            "message",
            "issues/items/properties/path",
            "issues/items/properties/message",
            "issues/items/properties/code",
        ] {
            field(
                &manifest["errorFrame"],
                "errorFrame",
                &format!("properties/{key}"),
                Some,
            )?;
        }

        let events = manifest["events"]
            .as_object_mut()
            .ok_or("events is missing")?;
        for (name, entry) in events.iter_mut() {
            self.load_event(name, entry)?;
        }

        let commands = manifest["commands"]
            .as_object_mut()
            .ok_or("commands is missing")?;
        for (name, entry) in commands.iter_mut() {
            self.load_command(name, entry)?;
        }

        Ok(())
    }

    fn load_descriptor(&mut self, descriptor: &mut Value) -> Result<(), String> {
        let properties = descriptor["properties"]
            .as_object_mut()
            .ok_or("descriptor.properties is missing")?;
        properties
            .remove("capabilities")
            .ok_or("the SDK declares descriptor.capabilities, but the protocol has none")?;
        if let Some(required) = descriptor["required"].as_array_mut() {
            required.retain(|name| name != "capabilities");
        }
        descriptor["description"] =
            "Source identity presented to Simlink. The SDK declares the capabilities.".into();
        self.root(Some("SourceOptions".into()), descriptor)?;

        Ok(())
    }

    fn load_event(&mut self, name: &str, entry: &mut Value) -> Result<(), String> {
        let channel = channel(name, entry)?;
        let payload = self.root(
            name_if_object(&entry["payload"], &pascal(name)),
            &mut entry["payload"],
        )?;
        self.events.push(EventChannel {
            channel,
            max_rate_hz: entry["maxRateHz"].as_f64(),
            payload,
        });

        Ok(())
    }

    fn load_command(&mut self, name: &str, entry: &mut Value) -> Result<(), String> {
        let channel = channel(name, entry)?;
        let timeout_ms = field(entry, name, "timeoutMs", Value::as_u64)?;
        let base = pascal(name);
        let input = self.root(Some(format!("{base}Input")), &mut entry["input"])?;
        let output = self.root(
            name_if_object(&entry["output"], &format!("{base}Output")),
            &mut entry["output"],
        )?;

        // A single refusal code is a `const`; as a one-value enum it keeps its own type.
        let errors = &mut entry["errors"];
        if let Some(value) = errors
            .as_object_mut()
            .and_then(|errors| errors.remove("const"))
        {
            errors["enum"] = Value::Array(vec![value]);
        }
        let error = self.root(Some(format!("{base}Error")), errors)?;

        self.commands.push(CommandChannel {
            channel,
            timeout_ms,
            input,
            output,
            error,
        });

        Ok(())
    }

    /// The generated types, and the Rust type of each root by index.
    fn types(&self) -> Result<(TokenStream, Vec<TokenStream>), String> {
        let mut settings = TypeSpaceSettings::default();
        settings.with_struct_builder(false);
        let mut space = TypeSpace::new(&settings);

        let defs = self
            .defs
            .iter()
            .map(|(name, schema)| {
                Ok((
                    name.clone(),
                    serde_json::from_value(schema.clone()).map_err(|e| e.to_string())?,
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        space.add_ref_types(defs).map_err(|e| e.to_string())?;

        let mut idents = Vec::new();
        for root in &self.roots {
            let schema: schemars::schema::Schema =
                serde_json::from_value(root.schema.clone()).map_err(|e| e.to_string())?;
            let id = match &root.name {
                Some(name) => space.add_type_with_name(&schema, Some(name.clone())),
                None => space.add_type(&schema),
            }
            .map_err(|e| e.to_string())?;
            idents.push(space.get_type(&id).map_err(|e| e.to_string())?.ident());
        }

        let mut file: syn::File = syn::parse2(space.to_stream()).map_err(|e| e.to_string())?;
        for item in &mut file.items {
            match item {
                syn::Item::Struct(item) => item
                    .attrs
                    .push(syn::parse_quote!(#[derive(::uniffi::Record)])),
                syn::Item::Enum(item) => item
                    .attrs
                    .push(syn::parse_quote!(#[derive(::uniffi::Enum)])),
                _ => {}
            }
        }

        Ok((quote!(#file), idents))
    }

    fn api(&self, idents: &[TokenStream]) -> TokenStream {
        let (variants, emits): (Vec<_>, Vec<_>) = self
            .events
            .iter()
            .map(|event| {
                let Channel { name, doc, deprecated } = &event.channel;
                let variant = format_ident!("{}", pascal(name));
                let method = format_ident!("emit_{}", snake(name));
                let payload = &idents[event.payload];
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

        let commands = self.commands.iter().map(|command| {
            let Channel {
                name,
                doc,
                deprecated,
            } = &command.channel;
            let base = pascal(name);
            let (handler, reply) = (format_ident!("{base}Handler"), format_ident!("{base}Reply"));
            let method = format_ident!("on_{}", snake(name));
            let input = &idents[command.input];
            let output = &idents[command.output];
            let error = &idents[command.error];
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
        });

        quote! {
            use crate::Source;

            /// Event channels a source can declare.
            #[derive(Clone, Copy, Debug, PartialEq, Eq, ::uniffi::Enum)]
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

    /// `HandshakeError`, `RejectionCode` and `Channel`, each with an `Other` variant for what a newer Simlink sends.
    fn codes(&self) -> TokenStream {
        let handshake_error = code_enum(
            "HandshakeError",
            "Why Simlink refused the handshake.",
            &self.handshake_errors,
        );
        let rejection_code = code_enum(
            "RejectionCode",
            "Why Simlink rejected a payload or answer.",
            &self.rejection_codes,
        );

        let channels = self
            .events
            .iter()
            .map(|event| &event.channel)
            .chain(self.commands.iter().map(|command| &command.channel));
        let (variants, reads, names): (Vec<_>, Vec<_>, Vec<_>) = channels
            .map(|Channel { name, doc, .. }| {
                let variant = format_ident!("{}", pascal(name));
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

    fn tables(&self, connection: &Value) -> Result<TokenStream, String> {
        let number = |key| field(connection, "connection", key, Value::as_u64);
        let text = |key| field(connection, "connection", key, Value::as_str);

        let protocol_version = number("protocolVersion")? as u32;
        let default_port = number("defaultPort")? as u16;
        let port_probe_limit = number("portProbeLimit")? as u16;
        let max_payload_bytes = number("socketIo/maxPayloadBytes")? as usize;
        let namespace = text("namespace")?;
        let mdns_service_type = text("mdnsServiceType")?;
        let error_channel = text("errorChannel")?;
        let socket_io_path = text("socketIo/path")?;
        // Discovery reads this TXT key by name.
        field(
            connection,
            "connection",
            "mdnsTxt/properties/protocol",
            Some,
        )?;

        let events = self.events.iter().map(|event| {
            let name = &event.channel.name;
            let rate = match event.max_rate_hz {
                Some(rate) => quote!(Some(#rate)),
                None => quote!(None),
            };
            quote!(crate::EventInfo { name: #name, max_rate_hz: #rate })
        });

        Ok(quote! {
            /// Wire protocol major. Simlink serves `/v{PROTOCOL_VERSION}/source` and advertises it in mDNS TXT.
            pub const PROTOCOL_VERSION: u32 = #protocol_version;
            pub(crate) const NAMESPACE: &str = #namespace;
            pub(crate) const DEFAULT_PORT: u16 = #default_port;
            pub(crate) const PORT_PROBE_LIMIT: u16 = #port_probe_limit;
            pub(crate) const MDNS_SERVICE_TYPE: &str = #mdns_service_type;
            pub(crate) const ERROR_CHANNEL: &str = #error_channel;
            pub(crate) const SOCKET_IO_PATH: &str = #socket_io_path;
            /// Simlink drops the connection on a larger message.
            pub(crate) const MAX_PAYLOAD_BYTES: usize = #max_payload_bytes;
            pub(crate) const EVENTS: &[crate::EventInfo] = &[#(#events),*];
        })
    }
}

/// Fails on any manifest key the SDK neither reads nor ignores on purpose, so a sync can't add something the SDK
/// silently skips. Schemas the macro hands to typify are left out: their types follow the manifest.
fn check_keys(manifest: &Value) -> Result<(), String> {
    known_keys(
        manifest,
        "manifest",
        &[
            "connection",
            "descriptor",
            "handshakeErrors",
            "errorFrame",
            "events",
            "commands",
        ],
    )?;
    let connection = &manifest["connection"];
    known_keys(
        connection,
        "connection",
        &[
            "protocolVersion",
            "defaultPort",
            "portProbeLimit",
            "mdnsServiceType",
            "mdnsTxt",
            "socketIo",
            "namespace",
            "errorChannel",
            // Ignored: how Simlink names its own mDNS advertisement, which discovery finds by service type.
            "mdnsInstanceName",
            "mdnsHostname",
            // Ignored: an HTTP header Node clients may send the descriptor in instead of the auth payload.
            "authHeader",
        ],
    )?;
    known_keys(
        &connection["socketIo"],
        "connection.socketIo",
        &[
            "path",
            "maxPayloadBytes",
            // Ignored: the client takes these from the Engine.IO open packet.
            "pingIntervalMs",
            "pingTimeoutMs",
        ],
    )?;
    known_keys(
        &connection["mdnsTxt"]["properties"],
        "connection.mdnsTxt.properties",
        &[
            "protocol", // Ignored: informational.
            "version",
        ],
    )?;

    for error in manifest["handshakeErrors"]["oneOf"]
        .as_array()
        .ok_or("handshakeErrors.oneOf is missing")?
    {
        known_keys(
            &error["properties"],
            "handshakeErrors.oneOf.properties",
            &["code", "metadata"],
        )?;
    }
    let frame = &manifest["errorFrame"]["properties"];
    known_keys(
        frame,
        "errorFrame.properties",
        &["event", "code", "message", "issues"],
    )?;
    known_keys(
        &frame["issues"]["items"]["properties"],
        "errorFrame.properties.issues.items.properties",
        &["code", "message", "path"],
    )?;

    for (name, entry) in object(&manifest["events"], "events")? {
        known_keys(
            entry,
            &format!("events.{name}"),
            &[
                "origin",
                "description",
                "deprecated",
                "maxRateHz",
                "payload",
            ],
        )?;
    }
    for (name, entry) in object(&manifest["commands"], "commands")? {
        known_keys(
            entry,
            &format!("commands.{name}"),
            &[
                "origin",
                "description",
                "deprecated",
                "timeoutMs",
                "input",
                "output",
                "errors",
                // Ignored: the reply envelope, which the SDK writes and the tests check replies against.
                "result",
            ],
        )?;
    }

    Ok(())
}

fn object<'a>(value: &'a Value, at: &str) -> Result<&'a Map<String, Value>, String> {
    value.as_object().ok_or(format!("{at} is missing"))
}

fn known_keys(value: &Value, at: &str, known: &[&str]) -> Result<(), String> {
    match object(value, at)?
        .keys()
        .find(|key| !known.contains(&key.as_str()))
    {
        Some(key) => Err(format!(
            "the manifest has {at}.{key}, which the SDK doesn't handle yet; read it in the macro, or add it to the \
             keys ignored on purpose in check_keys"
        )),
        None => Ok(()),
    }
}

/// A `handshakeErrors` entry: its code, and metadata fields of the types the SDK maps.
fn handshake_error(error: &Value) -> Result<Code, String> {
    let code = field(
        error,
        "handshakeErrors",
        "properties/code/const",
        Value::as_str,
    )?;
    let doc = error["properties"]["code"]["description"]
        .as_str()
        .unwrap_or_default();
    let metadata = &error["properties"]["metadata"];
    let required = metadata["required"].as_array().cloned().unwrap_or_default();
    let properties = metadata["properties"]
        .as_object()
        .cloned()
        .unwrap_or_default();

    let fields = properties
        .iter()
        .map(|(key, schema)| {
            let ty = metadata_type(schema).ok_or(format!(
                "handshakeErrors {code} has metadata.{key} of a type the SDK doesn't map yet"
            ))?;
            let ty = if required.iter().any(|name| name == key) {
                ty
            } else {
                quote!(Option<#ty>)
            };
            Ok((key.clone(), format_ident!("{}", snake_case(key)), ty))
        })
        .collect::<Result<_, String>>()?;

    new_code(code.to_owned(), "HANDSHAKE_", doc.to_owned(), fields)
}

/// `HANDSHAKE_SOURCE_ID_IN_USE` → `SourceIdInUse`, without `prefix`.
fn new_code(
    code: String,
    prefix: &str,
    doc: String,
    fields: Vec<(String, syn::Ident, TokenStream)>,
) -> Result<Code, String> {
    let name = pascal(
        &code
            .strip_prefix(prefix)
            .unwrap_or(&code)
            .to_ascii_lowercase(),
    );
    if name.is_empty() || name == "Other" {
        return Err(format!("{code} can't name a variant"));
    }

    Ok(Code {
        variant: format_ident!("{name}"),
        code,
        doc,
        fields,
    })
}

/// The Rust type of a metadata field, for the JSON types every binding has.
fn metadata_type(schema: &Value) -> Option<TokenStream> {
    Some(match schema["type"].as_str()? {
        "string" => quote!(String),
        "integer" => quote!(i64),
        "number" => quote!(f64),
        "boolean" => quote!(bool),
        "array" => {
            let item = metadata_type(&schema["items"])?;
            quote!(Vec<#item>)
        }
        _ => return None,
    })
}

/// An enum of `codes` read from a code and its metadata, with `Other` for codes newer than this SDK.
fn code_enum(name: &str, doc: &str, codes: &[Code]) -> TokenStream {
    let name = format_ident!("{name}");
    let variants = codes.iter().map(|code| {
        let (doc, variant) = (&code.doc, &code.variant);
        let fields = code.fields.iter().map(|(_, field, ty)| quote!(#field: #ty));
        if code.fields.is_empty() {
            quote!(#[doc = #doc] #variant)
        } else {
            quote!(#[doc = #doc] #variant { #(#fields),* })
        }
    });
    let reads = codes.iter().map(|code| {
        let (literal, variant) = (&code.code, &code.variant);
        let fields = code
            .fields
            .iter()
            .map(|(key, field, _)| quote!(#field: field(metadata, #key)));
        if code.fields.is_empty() {
            quote!(#literal => Self::#variant)
        } else {
            quote!(#literal => Self::#variant { #(#fields),* })
        }
    });
    let literals = codes.iter().map(|code| {
        let (literal, variant) = (&code.code, &code.variant);
        quote!(Self::#variant { .. } => #literal)
    });

    quote! {
        #[doc = #doc]
        #[derive(Clone, Debug, PartialEq, ::uniffi::Enum)]
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

/// What the generated API needs from a channel entry. The SDK only emits and answers on behalf of a source; other
/// channels would point the wrong way.
fn channel(name: &str, entry: &Value) -> Result<Channel, String> {
    match entry["origin"].as_str() {
        Some("source") => {}
        origin => {
            return Err(format!(
                "{name} has origin {origin:?}; the SDK only handles source channels"
            ));
        }
    }
    let deprecated = match &entry["deprecated"] {
        Value::String(reason) => Some(reason.clone()),
        Value::Bool(true) => Some(String::new()),
        _ => None,
    };

    Ok(Channel {
        name: name.to_owned(),
        doc: entry["description"].as_str().unwrap_or_default().to_owned(),
        deprecated,
    })
}

/// `key`, a `/`-separated path below `value`, read as `read` wants it. `at` names `value` in the error.
fn field<'a, T>(
    value: &'a Value,
    at: &str,
    key: &str,
    read: impl FnOnce(&'a Value) -> Option<T>,
) -> Result<T, String> {
    value
        .pointer(&format!("/{key}"))
        .and_then(read)
        .ok_or_else(|| format!("{at}.{} is missing", key.replace('/', ".")))
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

/// Objects get a channel-derived name; arrays and maps stay `Vec<T>` and `HashMap<String, T>`.
fn name_if_object(schema: &Value, name: &str) -> Option<String> {
    schema.get("properties").map(|_| name.to_owned())
}

/// `telemetry.ownship` → `telemetry`, `ownship`.
fn words(name: &str) -> impl Iterator<Item = &str> {
    name.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
}

/// `telemetry.ownship` → `TelemetryOwnship`.
fn pascal(name: &str) -> String {
    words(name)
        .map(|word| word[..1].to_ascii_uppercase() + &word[1..])
        .collect()
}

/// `telemetry.ownship` → `telemetry_ownship`.
fn snake(name: &str) -> String {
    words(name).collect::<Vec<_>>().join("_")
}

/// `sourceId` → `source_id`.
fn snake_case(name: &str) -> String {
    let mut snake = String::new();
    for c in name.chars() {
        if c.is_ascii_uppercase() && !snake.is_empty() {
            snake.push('_');
        }
        snake.push(c.to_ascii_lowercase());
    }
    snake
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn rejects_manifest_keys_the_sdk_does_not_handle() {
        let text = include_str!("../../../protocol/source-schema.json");
        let manifest: Value = serde_json::from_str(text).unwrap();
        assert_eq!(check_keys(&manifest), Ok(()));

        for pointer in [
            "",
            "/connection",
            "/events/telemetry.ownship",
            "/errorFrame/properties",
        ] {
            let mut changed = manifest.clone();
            changed.pointer_mut(pointer).unwrap()["unknown"] = json!(1);
            let error = check_keys(&changed).unwrap_err();
            assert!(error.contains(".unknown,"), "{pointer}: {error}");
        }
    }

    #[test]
    fn simplify_strips_keywords_but_keeps_properties_named_like_them() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "format": { "type": "string", "format": "date-time" },
                "pattern": { "type": "string", "pattern": "^a$", "maxLength": 4 },
                "kind": { "type": "string", "const": "standard", "default": "standard" },
            },
            "required": ["format", "kind"],
            "$defs": { "default": { "type": "string", "minLength": 1 } },
        });
        simplify(&mut schema);

        assert_eq!(
            schema,
            json!({
                "type": "object",
                "properties": { "format": { "type": "string" }, "pattern": { "type": "string" } },
                "required": ["format"],
                "$defs": { "default": { "type": "string" } },
            })
        );
    }
}
