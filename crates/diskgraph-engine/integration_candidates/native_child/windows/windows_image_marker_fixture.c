#define WIN32_LEAN_AND_MEAN
#include <windows.h>

/* 同一源与工具链构建 A/B；只改变真实 PE 中等长的 volatile marker。 */
#ifdef IMAGE_B
static volatile char image_marker[] = "DISKGRAPH_IMAGE_B_LEASE_MARKER";
#else
static volatile char image_marker[] = "DISKGRAPH_IMAGE_A_LEASE_MARKER";
#endif

int main(void) {
    char output[sizeof(image_marker)];
    DWORD written = 0;
    SIZE_T i;
    for (i = 0; i + 1 < sizeof(image_marker); ++i) {
        output[i] = image_marker[i];
    }
    output[sizeof(image_marker) - 1] = '\n';
    if (!WriteFile(GetStdHandle(STD_OUTPUT_HANDLE), output,
                   (DWORD)sizeof(output), &written, NULL)) {
        return 90;
    }
    return written == sizeof(output) ? 0 : 91;
}
