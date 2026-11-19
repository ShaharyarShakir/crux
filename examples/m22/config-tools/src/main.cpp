#include <cxxopts.hpp>
#include <toml++/toml.hpp>
#include <yaml-cpp/yaml.h>

#include <iostream>
#include <string>

int main(int argc, char** argv) {
    cxxopts::Options options{"config-tools", "M22 tomlplusplus + cxxopts + yaml-cpp reference"};
    options.add_options()("n,name", "Configuration name", cxxopts::value<std::string>()->default_value("crux"));
    const auto parsed = options.parse(argc, argv);
    const std::string name = parsed["name"].as<std::string>();

    const auto toml = toml::parse(R"([app]
enabled = true
threads = 4
)");
    const auto threads = toml["app"]["threads"].value<int>().value_or(0);

    const YAML::Node yaml = YAML::Load("runtime: linux\narch: x86_64\n");

    std::cout << "name=" << name << '\n';
    std::cout << "threads=" << threads << '\n';
    std::cout << "runtime=" << yaml["runtime"].as<std::string>() << '\n';
    std::cout << "arch=" << yaml["arch"].as<std::string>() << '\n';
    return 0;
}
