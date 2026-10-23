// Answers Simlink's ATIS command, refusing airports without one. Stop with Ctrl+C.
#include <chrono>
#include <cstdio>
#include <optional>
#include <thread>

#include <simlink.hpp>

// The airport's current ATIS broadcasts, if it has any.
std::optional<std::vector<simlink::AtisReport>> atis(const std::string& icao) {
    if (icao != "ESSA") return std::nullopt;

    simlink::AtisReport arrival{};
    arrival.icao = icao;
    arrival.type = simlink::AtisReportType::kArr;
    arrival.letter = "A";
    arrival.phonetic = "Alpha";
    arrival.message = "Stockholm Arlanda arrival information Alpha";
    arrival.date_issued = "2026-10-19T12:00:00Z";
    arrival.arriving_runways = {"19R"};

    simlink::AtisReport departure{};
    departure.icao = icao;
    departure.type = simlink::AtisReportType::kDep;
    departure.letter = "B";
    departure.phonetic = "Bravo";
    departure.message = "Stockholm Arlanda departure information Bravo";
    departure.date_issued = "2026-10-19T12:00:00Z";
    departure.departing_runways = {"19L"};

    return std::vector{arrival, departure};
}

struct Atis : simlink::AtisGetHandler {
    void handle(const simlink::AtisGetInput& input,
                const std::shared_ptr<simlink::AtisGetReply>& reply) override {
        // Answering twice throws simlink::SimlinkError.
        try {
            if (auto reports = atis(input.icao)) {
                reply->ok(*reports);
            } else {
                reply->refuse(simlink::AtisGetError::kAtisNotAvailable);
            }
        } catch (const simlink::SimlinkError& error) {
            std::fprintf(stderr, "Could not answer atis.get: %s\n", error.what());
        }
    }
};

int main() {
    simlink::SourceOptions options{};
    options.id = "commands";
    options.display_name = "Commands";

    // Commands are declared by registering their handlers, so this source declares no events.
    auto source = simlink::Source::init(options, {});
    source->on_atis_get(std::make_shared<Atis>());
    source->start();

    for (;;) std::this_thread::sleep_for(std::chrono::seconds(1));
}
