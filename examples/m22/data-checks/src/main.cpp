#include <cjson/cJSON.h>

#define PCRE2_CODE_UNIT_WIDTH 8
#include <pcre2.h>

#include <yyjson.h>

#include <iostream>
#include <string>

int main() {
    constexpr char kDocument[] = R"({"milestone":"M22","packages":50})";

    cJSON* parsed = cJSON_Parse(kDocument);
    if (parsed == nullptr) {
        std::cerr << "cJSON failed to parse the document\n";
        return 1;
    }
    const int cjson_packages = cJSON_GetObjectItemCaseSensitive(parsed, "packages")->valueint;
    cJSON_Delete(parsed);

    yyjson_doc* document = yyjson_read(kDocument, sizeof(kDocument) - 1, 0);
    if (document == nullptr) {
        std::cerr << "yyjson failed to parse the document\n";
        return 1;
    }
    yyjson_val* root = yyjson_doc_get_root(document);
    const int yyjson_packages = static_cast<int>(yyjson_get_int(yyjson_obj_get(root, "packages")));
    yyjson_doc_free(document);

    int error_code = 0;
    PCRE2_SIZE error_offset = 0;
    pcre2_code* regex = pcre2_compile(
        reinterpret_cast<PCRE2_SPTR>("^crux-(\\d+)$"),
        PCRE2_ZERO_TERMINATED,
        0,
        &error_code,
        &error_offset,
        nullptr
    );
    if (regex == nullptr) {
        std::cerr << "pcre2 failed to compile the pattern\n";
        return 1;
    }
    pcre2_match_data* match_data = pcre2_match_data_create_from_pattern(regex, nullptr);
    constexpr char kSubject[] = "crux-22";
    const bool matched = pcre2_match(
        regex,
        reinterpret_cast<PCRE2_SPTR>(kSubject),
        sizeof(kSubject) - 1,
        0,
        0,
        match_data,
        nullptr
    ) > 0;

    pcre2_match_data_free(match_data);
    pcre2_code_free(regex);

    std::cout << "cJSON packages=" << cjson_packages << '\n';
    std::cout << "yyjson packages=" << yyjson_packages << '\n';
    std::cout << "pcre2 matched=" << std::boolalpha << matched << '\n';
    return (cjson_packages == 50 && yyjson_packages == 50 && matched) ? 0 : 1;
}
