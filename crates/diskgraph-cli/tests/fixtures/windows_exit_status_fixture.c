/* Windows 原始退出状态资格夹具。来源：CLI serve 的 ExitStatus 转交契约。
 * 仅按编译时固定的两个常量调用真实 ExitProcess，不模拟 MCP 或任何授权能力。
 */
#define WIN32_LEAN_AND_MEAN
#include <windows.h>

#ifndef FIXTURE_EXIT_CODE
#error FIXTURE_EXIT_CODE must be supplied explicitly for a native fixture build
#endif

#if FIXTURE_EXIT_CODE != 0xC0000000UL && FIXTURE_EXIT_CODE != 0xC0000005UL
#error FIXTURE_EXIT_CODE must be one of the two contract qualification constants
#endif

int main(void) {
    ExitProcess((UINT)FIXTURE_EXIT_CODE);
}
