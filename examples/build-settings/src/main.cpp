#include <iostream>
#include <vector>

int main() {
    std::vector<int> values{1, 2, 3};
    int sum = 0;
    for (int v : values) {
        sum += v;
    }
    std::cout << "sum = " << sum << "\n";
    // Uncomment to see AddressSanitizer report a heap-buffer-overflow:
    // std::cout << values.data()[values.size() + 1] << "\n";
    return 0;
}
