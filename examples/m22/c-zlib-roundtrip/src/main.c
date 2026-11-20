#include <zlib.h>

#include <stdio.h>
#include <string.h>

int main(void) {
    const char message[] = "Crux M22 pure C + verified zlib";
    const size_t message_len = strlen(message);
    Bytef compressed[256];
    Bytef restored[256];
    uLongf compressed_len = (uLongf)sizeof(compressed);
    uLongf restored_len = (uLongf)sizeof(restored);

    int status = compress2(
        compressed,
        &compressed_len,
        (const Bytef *)message,
        (uLong)message_len,
        Z_BEST_COMPRESSION
    );
    if (status != Z_OK) {
        fprintf(stderr, "zlib compression failed with status %d\n", status);
        return 1;
    }

    status = uncompress(restored, &restored_len, compressed, compressed_len);
    if (status != Z_OK) {
        fprintf(stderr, "zlib decompression failed with status %d\n", status);
        return 1;
    }

    const int roundtrip_ok = restored_len == message_len &&
                             memcmp(restored, message, message_len) == 0;
    printf("roundtrip_ok=%s\n", roundtrip_ok ? "true" : "false");
    printf("compressed_bytes=%lu\n", (unsigned long)compressed_len);
    return roundtrip_ok ? 0 : 1;
}
