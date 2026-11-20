#include <spdlog/spdlog.h>
#include <zlib.h>

#include <cstring>
#include <iostream>
#include <string>
#include <vector>

int main() {
    const std::string message = "Crux M22 verified spdlog and zlib";
    std::vector<unsigned char> compressed(compressBound(static_cast<uLong>(message.size())));
    uLongf compressed_size = static_cast<uLongf>(compressed.size());

    const int compress_status = compress2(
        compressed.data(),
        &compressed_size,
        reinterpret_cast<const Bytef*>(message.data()),
        static_cast<uLong>(message.size()),
        Z_BEST_COMPRESSION
    );
    if (compress_status != Z_OK) {
        std::cerr << "zlib compression failed with status " << compress_status << '\n';
        return 1;
    }

    std::vector<unsigned char> restored(message.size());
    uLongf restored_size = static_cast<uLongf>(restored.size());
    const int uncompress_status = uncompress(
        restored.data(),
        &restored_size,
        compressed.data(),
        compressed_size
    );
    if (uncompress_status != Z_OK) {
        std::cerr << "zlib decompression failed with status " << uncompress_status << '\n';
        return 1;
    }

    const std::string roundtrip(
        reinterpret_cast<const char*>(restored.data()),
        static_cast<std::size_t>(restored_size)
    );
    spdlog::set_level(spdlog::level::info);
    spdlog::info("roundtrip_ok={}", roundtrip == message);
    spdlog::info("compressed_bytes={}", compressed_size);

    std::cout << "roundtrip_ok=" << std::boolalpha << (roundtrip == message) << '\n';
    std::cout << "compressed_bytes=" << compressed_size << '\n';
    return roundtrip == message ? 0 : 1;
}
