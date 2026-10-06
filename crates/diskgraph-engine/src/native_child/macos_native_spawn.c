/*
 * PF-06 macOS 公开 posix_spawn adapter；只负责原子出生与首个外部 PID owner。
 * 来源：Apple SDK spawn.h / sys/spawn.h；无 fork、宿主 pre_exec 或私有 SPI。
 * 最低资格保守要求 macOS 11+；SDK 宏存在不替代 Rust 的实际 SID/PGID 核验。
 */
#ifndef _DARWIN_C_SOURCE
#define _DARWIN_C_SOURCE 1
#endif

#include <errno.h>
#include <signal.h>
#include <spawn.h>
#include <stdbool.h>
#include <stddef.h>
#include <sys/types.h>
#include <unistd.h>

#if !defined(POSIX_SPAWN_SETSID) || !defined(POSIX_SPAWN_CLOEXEC_DEFAULT)
#error "macOS public spawn session and explicit descriptor inheritance are required"
#endif

/*
 * 参数：path 是 Rust 已验证且 lease 保留的绝对路径，不读取 PATH 或动态 argv/env。
 * input/output/diagnostic 是调用方独占、互异且至少为 3 的已持有子端 FD；本函数
 * 不关闭父端原 FD。owned_pid/owns_group 是出生前初始化为 0/false 的原外部 owner。
 * 返回：0 只表示已出生且原 PID/group 责任已转交；非零保留出生前原 errno。
 * 空环境固定；cwd 仍继承，加载依赖与 cwd 安全属于可信安装的独立资格。
 * 不安装 RLIMIT、不证明镜像加载/无后代、完整协议或组退出；Rust 必须继续核验。
 */
int diskgraph_macos_native_spawn(const char *path, int input, int output,
                                int diagnostic, pid_t *owned_pid,
                                bool *owns_group) {
    posix_spawnattr_t attributes = NULL;
    posix_spawn_file_actions_t actions = NULL;
    sigset_t mask, defaults;
    const int reset_signals[] = {SIGCHLD, SIGPIPE, SIGINT, SIGTERM, SIGHUP, SIGQUIT};
    char *const arguments[] = {(char *)"diskgraph-scan-worker", NULL};
    char *const environment[] = {NULL};
    bool attributes_ready = false, actions_ready = false, born = false;
    int error = 0, secondary;
    size_t index;

    /* 只准入空 owner；失败不清零或覆盖调用方已持有的另一代资源。 */
    if (path == NULL || path[0] != '/' || owned_pid == NULL || owns_group == NULL ||
        input < 3 || output < 3 || diagnostic < 3 || input == output ||
        input == diagnostic || output == diagnostic) {
        return EINVAL;
    }
    if (*owned_pid != 0 || *owns_group) {
        return EINVAL;
    }

    error = posix_spawnattr_init(&attributes);
    if (error != 0) goto finish;
    attributes_ready = true;
    error = posix_spawn_file_actions_init(&actions);
    if (error != 0) goto finish;
    actions_ready = true;

    /* 只设置子进程属性，不修改宿主的 signal mask/handler 或全局 SIGPIPE 策略。 */
    if (sigemptyset(&mask) != 0 || sigemptyset(&defaults) != 0) {
        error = errno;
        goto finish;
    }
    for (index = 0; index < sizeof(reset_signals) / sizeof(reset_signals[0]); ++index) {
        if (sigaddset(&defaults, reset_signals[index]) != 0) {
            error = errno;
            goto finish;
        }
    }
    error = posix_spawnattr_setsigmask(&attributes, &mask);
    if (error != 0) goto finish;
    error = posix_spawnattr_setsigdefault(&attributes, &defaults);
    if (error != 0) goto finish;
    /* XNU 先处理 SETPGROUP 再处理 SETSID；本入口只请求新 session。 */
    error = posix_spawnattr_setflags(
        &attributes, (short)(POSIX_SPAWN_SETSID | POSIX_SPAWN_CLOEXEC_DEFAULT |
                             POSIX_SPAWN_SETSIGMASK | POSIX_SPAWN_SETSIGDEF));
    if (error != 0) goto finish;

    error = posix_spawn_file_actions_adddup2(&actions, input, STDIN_FILENO);
    if (error != 0) goto finish;
    error = posix_spawn_file_actions_adddup2(&actions, output, STDOUT_FILENO);
    if (error != 0) goto finish;
    error = posix_spawn_file_actions_adddup2(&actions, diagnostic, STDERR_FILENO);
    if (error != 0) goto finish;
    /* 原描述符都 >=3 且互异，关闭动作不会覆盖映射后的三个标准通道。 */
    error = posix_spawn_file_actions_addclose(&actions, input);
    if (error != 0) goto finish;
    error = posix_spawn_file_actions_addclose(&actions, output);
    if (error != 0) goto finish;
    error = posix_spawn_file_actions_addclose(&actions, diagnostic);
    if (error != 0) goto finish;

    /* 返回值本身是 errno；Darwin wrapper 会恢复宿主 errno，不能重新读 errno。 */
    error = posix_spawn(owned_pid, path, &actions, &attributes, arguments, environment);
    if (error == 0) {
        /* 出生后无分配、查询或回调；先保留原 owner，再销毁预备对象。 */
        *owns_group = true;
        born = true;
    } else {
        /* 公开 API 非零返回不创建 child，失败时 PID 输出未定义，恢复原空槽。 */
        *owned_pid = 0;
    }

finish:
    if (actions_ready) {
        secondary = posix_spawn_file_actions_destroy(&actions);
        if (!born && error == 0 && secondary != 0) error = secondary;
    }
    if (attributes_ready) {
        secondary = posix_spawnattr_destroy(&attributes);
        if (!born && error == 0 && secondary != 0) error = secondary;
    }
    /* 成功出生不可被 destroy 次错改记为无 child；首个 owner 始终由 Rust 接管。 */
    return error;
}
