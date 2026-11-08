//! Answers Simlink's ATIS command, refusing airports without one. Stop with Ctrl+C.

use std::sync::Arc;
use std::time::Duration;

use simlink_source_sdk::*;

struct Atis;

impl AtisGetHandler for Atis {
    fn handle(&self, input: AtisGetInput, reply: Arc<AtisGetReply>) {
        let answered = match atis(&input.icao) {
            Some(reports) => reply.ok(reports),
            None => reply.refuse(AtisGetError::AtisNotAvailable),
        };
        // Answering twice fails with `Error::AlreadyAnswered`.
        if let Err(error) = answered {
            eprintln!("Could not answer atis.get: {error}");
        }
    }
}

fn main() -> Result<(), Error> {
    let options = SourceOptions {
        id: "commands".into(),
        display_name: "Commands".into(),
    };
    // Commands are declared by registering their handlers, so this source declares no events.
    let source = Source::new(options, vec![]);
    source.on_atis_get(Arc::new(Atis))?;
    source.start()?;

    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// The airport's current ATIS broadcasts, if it has any.
fn atis(icao: &str) -> Option<Vec<AtisReport>> {
    (icao == "ESSA").then(|| {
        vec![
            AtisReport {
                icao: icao.into(),
                type_: AtisReportType::Arr,
                letter: Some("A".into()),
                phonetic: Some("Alpha".into()),
                message: "Stockholm Arlanda arrival information Alpha".into(),
                date_issued: "2026-10-19T12:00:00Z".into(),
                arriving_runways: vec!["19R".into()],
                departing_runways: vec![],
            },
            AtisReport {
                icao: icao.into(),
                type_: AtisReportType::Dep,
                letter: Some("B".into()),
                phonetic: Some("Bravo".into()),
                message: "Stockholm Arlanda departure information Bravo".into(),
                date_issued: "2026-10-19T12:00:00Z".into(),
                arriving_runways: vec![],
                departing_runways: vec!["19L".into()],
            },
        ]
    })
}
