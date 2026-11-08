//! Every example program, built against the bindings by `cargo xtask examples`, joins a test Simlink, emits valid
//! payloads on the events it declares, answers the commands it handles, and survives a rejection.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use serde_json::{Value, json};
use support::manifest::{MANIFEST, issues, sample};
use support::server::Server;

mod support;

// C# programs start the .NET runtime first.
const JOIN_TIMEOUT: Duration = Duration::from_secs(30);
const EVENT_TIMEOUT: Duration = Duration::from_secs(10);
const SURVIVAL: Duration = Duration::from_millis(500);

#[test]
#[ignore = "needs the programs `cargo xtask examples` builds"]
fn examples_join_and_answer() {
    let programs = std::env::var_os("SIMLINK_SOURCE_SDK_EXAMPLES")
        .expect("SIMLINK_SOURCE_SDK_EXAMPLES lists the example programs");
    let failures: Vec<String> = std::env::split_paths(&programs)
        .filter_map(|program| {
            check(&program)
                .err()
                .map(|error| format!("{}: {error}", program.display()))
        })
        .collect();
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

fn check(program: &Path) -> Result<(), String> {
    // The examples use discovery, as real sources do.
    let server = Server::on_simlink_port()?;
    let result = run(program, &server);
    // Collected here rather than in `Drop`, so one program's failure doesn't hide the others'.
    match (result, server.finish().as_slice()) {
        (Ok(()), []) => Ok(()),
        (Ok(()), violations) => Err(format!("broke the protocol: {violations:?}")),
        (Err(error), _) => Err(error),
    }
}

fn run(program: &Path, server: &Server) -> Result<(), String> {
    let mut source = Running(
        Command::new(program)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?,
    );
    let output = lines(&mut source.0);

    let Some(descriptor) = server.wait_for_join(JOIN_TIMEOUT) else {
        return Err(match source.0.try_wait() {
            Ok(Some(status)) => format!("exited with {status} before joining"),
            _ => "never joined".into(),
        });
    };
    // Simlink refuses an invalid descriptor for good.
    valid("descriptor", &MANIFEST["descriptor"], &descriptor)?;
    let declared = |kind: &str| -> Vec<String> {
        let names = descriptor["capabilities"][kind]
            .as_array()
            .cloned()
            .unwrap_or_default();
        names
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect()
    };

    for event in declared("events") {
        let payload = server
            .wait_for_event(&event, EVENT_TIMEOUT)
            .ok_or(format!("emitted no {event}"))?;
        valid(
            &format!("{event} payload"),
            &MANIFEST["events"][&event]["payload"],
            &payload,
        )?;
    }

    for command in declared("commands") {
        let schemas = &MANIFEST["commands"][&command];
        let answer = server
            .command(
                &command,
                sample(&schemas["input"], &schemas["input"], false, 0),
            )
            .ok_or(format!("did not answer {command}"))?;
        valid(&format!("{command} answer"), &schemas["result"], &answer)?;
    }

    // Every example survives a rejection, and the diagnostics ones print it, issues included.
    let rejection = json!({
        "event": "telemetry.ownship",
        "code": "INVALID_DATA",
        "message": "test rejection",
        "issues": [{ "code": "custom", "message": "test issue", "path": ["aircraft", 0, "key"] }],
    });
    valid("error frame", &MANIFEST["errorFrame"], &rejection)?;
    server.send_error(rejection);
    let name = program.file_stem().unwrap_or_default().to_string_lossy();
    if name.eq_ignore_ascii_case("diagnostics") {
        let expected = "aircraft[0].key: test issue";
        std::iter::from_fn(|| output.recv_timeout(EVENT_TIMEOUT).ok())
            .find(|line| line.contains(expected))
            .ok_or(format!("did not print the rejection issue \"{expected}\""))?;
    }
    std::thread::sleep(SURVIVAL);
    if let Ok(Some(status)) = source.0.try_wait() {
        return Err(format!("exited with {status} after a rejection"));
    }

    Ok(())
}

/// The program's stdout, line by line.
fn lines(child: &mut Child) -> Receiver<String> {
    let (send, receive) = mpsc::channel();
    let stdout = child.stdout.take().expect("stdout is piped");
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if send.send(line).is_err() {
                break;
            }
        }
    });
    receive
}

fn valid(what: &str, schema: &Value, value: &Value) -> Result<(), String> {
    match issues(schema, value).as_slice() {
        [] => Ok(()),
        found => Err(format!("invalid {what}: {found:?}")),
    }
}

/// Kills the program when the check ends, however it ends.
struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
