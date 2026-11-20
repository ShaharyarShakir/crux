#define DOCTEST_CONFIG_IMPLEMENT
#define DOCTEST_CONFIG_NO_SHORT_MACRO_NAMES

#include <benchmark/benchmark.h>
#include <catch2/catch_test_macros.hpp>
#include <catch2/catch_session.hpp>
#include <doctest/doctest.h>
#include <gtest/gtest.h>

#include <cstdint>
#include <iostream>
#include <vector>

TEST(CruxM22, GoogleTestRuns) {
    EXPECT_EQ(22, 22);
}

DOCTEST_TEST_CASE("doctest runs") {
    DOCTEST_CHECK(22 == 22);
}

TEST_CASE("catch2 runs", "[m22]") {
    REQUIRE(22 == 22);
}

static void BM_Sum(benchmark::State& state) {
    std::uint64_t total = 0;
    for (auto _ : state) {
        for (int i = 0; i < 64; ++i) {
            total += static_cast<std::uint64_t>(i);
        }
        benchmark::DoNotOptimize(total);
    }
}
BENCHMARK(BM_Sum);

int main(int argc, char** argv) {
    Catch::Session catch_session;
    const int catch_status = catch_session.run(argc, argv);

    testing::InitGoogleTest(&argc, argv);
    const int gtest_status = RUN_ALL_TESTS();

    doctest::Context doctest_context;
    doctest_context.applyCommandLine(argc, argv);
    const int doctest_status = doctest_context.run();

    benchmark::Initialize(&argc, argv);
    benchmark::RunSpecifiedBenchmarks();
    benchmark::Shutdown();

    const bool benchmark_ready = true;

    std::cout << "catch2_status=" << catch_status << '\n';
    std::cout << "googletest_status=" << gtest_status << '\n';
    std::cout << "doctest_status=" << doctest_status << '\n';
    std::cout << "benchmark_ready=" << std::boolalpha << benchmark_ready << '\n';

    return (catch_status == 0 && gtest_status == 0 && doctest_status == 0 && benchmark_ready) ? 0 : 1;
}
