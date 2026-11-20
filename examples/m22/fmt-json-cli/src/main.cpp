#include <CLI/CLI.hpp>
#include <fmt/core.h>
#include <nlohmann/json.hpp>

int main(int argc, char** argv) {
    CLI::App app{"M22 fmt + nlohmann-json + CLI11 reference"};
    std::string name = "Crux";
    app.add_option("-n,--name", name, "Name to include in the JSON document");
    CLI11_PARSE(app, argc, argv);

    const nlohmann::json document = {
        {"milestone", "M22"},
        {"package_manager", "Crux"},
        {"user", name},
    };

    fmt::print("{}\n", document.dump(2));
    return 0;
}
