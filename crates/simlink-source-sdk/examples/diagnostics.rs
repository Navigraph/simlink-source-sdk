//! Prints the SDK's logs, connection changes and anything Simlink rejects. Stop with Ctrl+C.

use std::sync::Arc;
use std::time::Duration;

use simlink_source_sdk::*;

struct Log;

impl LogListener for Log {
    fn log(&self, level: LogLevel, message: String) {
        eprintln!("[{level:?}] {message}");
    }
}

struct State;

impl StateListener for State {
    fn changed(&self, state: ConnectionState) {
        match state {
            ConnectionState::Searching => println!("Searching for Simlink"),
            ConnectionState::Connected { address } => println!("Connected to {address}"),
            ConnectionState::Rejected {
                code,
                error,
                message,
                retrying,
            } => {
                println!(
                    "Refused ({code}): {message}{}",
                    if retrying { ", retrying" } else { "" }
                );
                if let Some(HandshakeError::HandshakeIdentificationInvalid { issues }) = error {
                    for issue in issues {
                        println!("  {issue}");
                    }
                }
            }
        }
    }
}

struct Rejections;

impl RejectionListener for Rejections {
    fn rejected(&self, rejection: Rejection) {
        let Rejection {
            channel,
            code,
            message,
            issues,
        } = rejection;
        println!(
            "Simlink rejected {} ({code}): {}",
            channel.wire_name(),
            message.unwrap_or_default()
        );
        for issue in issues {
            println!("  {}: {}", issue.path, issue.message);
        }
    }
}

fn main() -> Result<(), Error> {
    set_logger(Arc::new(Log), LogLevel::Debug);

    let options = SourceOptions {
        id: "diagnostics".into(),
        display_name: "Diagnostics".into(),
    };
    let source = Source::new(options, vec![Event::TelemetryTraffic]);
    source.on_state(Arc::new(State))?;
    source.on_rejected(Arc::new(Rejections))?;
    source.start()?;

    loop {
        // An empty traffic picture is valid; a real source sends the aircraft around the user.
        // Emitting fails for an undeclared event, before `start`, or over Simlink's message size limit.
        if let Err(error) = source.emit_telemetry_traffic(vec![]) {
            println!("Could not emit: {error}");
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}
