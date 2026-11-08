// Answers Simlink's ATIS command, refusing airports without one. Stop with Ctrl+C.
#include <chrono>
#include <cstdio>
#include <optional>
#include <thread>

#include <simlink_source_sdk.hpp>

// The airport's current ATIS broadcasts, if it has any.
std::optional<std::vector<simlink_source_sdk::AtisReport>> atis(const std::string& icao) {
    if (icao != "ESSA") return std::nullopt;

    simlink_source_sdk::AtisReport arrival{};
    arrival.icao = icao;
    arrival.type = simlink_source_sdk::AtisReportType::kArr;
    arrival.letter = "A";
    arrival.phonetic = "Alpha";
    arrival.message = "Stockholm Arlanda arrival information Alpha";
    arrival.date_issued = "2026-10-19T12:00:00Z";
    arrival.arriving_runways = {"19R"};

    simlink_source_sdk::AtisReport departure{};
    departure.icao = icao;
    departure.type = simlink_source_sdk::AtisReportType::kDep;
    departure.letter = "B";
    departure.phonetic = "Bravo";
    departure.message = "Stockholm Arlanda departure information Bravo";
    departure.date_issued = "2026-10-19T12:00:00Z";
    departure.departing_runways = {"19L"};

    return std::vector{arrival, departure};
}

struct Atis : simlink_source_sdk::AtisGetHandler {
    void handle(const simlink_source_sdk::AtisGetInput& input,
                const std::shared_ptr<simlink_source_sdk::AtisGetReply>& reply) override {
        // Answering twice throws simlink_source_sdk::SimlinkError.
        try {
            if (auto reports = atis(input.icao)) {
                reply->ok(*reports);
            } else {
                reply->refuse(simlink_source_sdk::AtisGetError::kAtisNotAvailable);
            }
        } catch (const simlink_source_sdk::SimlinkError& error) {
            std::fprintf(stderr, "Could not answer atis.get: %s\n", error.what());
        }
    }
};

int main() {
    simlink_source_sdk::SourceOptions options{};
    options.id = "commands";
    options.display_name = "Commands";

    // Commands are declared by registering their handlers, so this source declares no events.
    auto source = simlink_source_sdk::Source::init(options, {});
    source->on_atis_get(std::make_shared<Atis>());
    source->start();

    for (;;) std::this_thread::sleep_for(std::chrono::seconds(1));
}
