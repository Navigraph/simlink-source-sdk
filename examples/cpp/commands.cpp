// Answers Simlink's ATIS and flight plan commands. Stop with Ctrl+C.
#include <chrono>
#include <thread>

#include <simlink.hpp>

struct Atis : simlink::AtisGetHandler {
    void handle(const simlink::AtisGetInput& input,
                const std::shared_ptr<simlink::AtisGetReply>& reply) override {
        if (input.icao != "ESSA") {
            reply->refuse(simlink::AtisGetError::kAtisNotAvailable);
            return;
        }

        simlink::AtisReport report{};
        report.icao = input.icao;
        report.type = simlink::AtisReportType::kArr;
        report.letter = "A";
        report.phonetic = "Alpha";
        report.message = "Stockholm Arlanda information Alpha";
        report.date_issued = "2026-10-19T12:00:00Z";
        report.arriving_runways = {"19R"};
        report.departing_runways = {"19L"};
        reply->ok({report});
    }
};

struct FlightPlans : simlink::FlightplanGetHandler {
    void handle(const simlink::FlightplanGetInput& input,
                const std::shared_ptr<simlink::FlightplanGetReply>& reply) override {
        // No flight plans here; a real source looks each key up.
        std::unordered_map<std::string, std::optional<simlink::FlightPlan>> plans;
        for (const auto& key : input.keys) plans[key] = std::nullopt;
        reply->ok(plans);
    }
};

int main() {
    simlink::SourceOptions options{};
    options.id = "commands";
    options.display_name = "Commands";

    auto source = simlink::Source::init(options, {});
    source->on_atis_get(std::make_shared<Atis>());
    source->on_flightplan_get(std::make_shared<FlightPlans>());
    source->start();

    for (;;) std::this_thread::sleep_for(std::chrono::seconds(1));
}
