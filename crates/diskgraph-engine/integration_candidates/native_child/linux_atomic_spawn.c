#define _GNU_SOURCE
#include "linux_atomic_syscall.h"
#include <errno.h>
#include <fcntl.h>
#include <linux/filter.h>
#include <linux/sched.h>
#include <signal.h>
#include <stddef.h>
#include <sys/prctl.h>
#include <sys/socket.h>
#include <sys/syscall.h>

/* 已准备材料的 native64 C ABI；所有 fd 源均 >=10，避免固定目标槽与源别名。 */
struct dg_request {
    int fds[5]; /* stdin/stdout/stderr/held ELF/startup socket */
    char *const *argv;
    char *const *envp;
    unsigned short filter_count;
    const struct sock_filter *filter;
};
struct dg_message { uint32_t kind, phase; int32_t error; };
struct dg_kernel_action { uintptr_t handler, flags, restorer; uint64_t mask; };
_Static_assert(sizeof(void *) == 8, "native64 pointers");
_Static_assert(sizeof(struct dg_kernel_action) == 32, "kernel rt_sigaction layout");
_Static_assert(sizeof(struct dg_message) == 12, "bounded startup message");
_Static_assert(offsetof(struct dg_request, argv) == 24, "request pointer offset");

static __attribute__((noreturn)) void dg_exit(void) {
    dg_raw(SYS_exit_group, 127, 0, 0, 0, 0, 0);
    for (;;) { /* exit_group 成功不返回；被宿主策略拒绝时不得返回 Rust/运行析构。 */ }
}

static __attribute__((noreturn)) void dg_fail(int fd, uint32_t phase, long result) {
    struct dg_message message = {2, phase, (int32_t)-result};
    /* 一笔固定栈消息，MSG_DONTWAIT 不因对端不读而卡住子初始化；退出由原 pidfd 观察。 */
    dg_raw(SYS_sendto, fd, (long)&message, sizeof(message),
        MSG_NOSIGNAL | MSG_DONTWAIT, 0, 0);
    dg_exit();
}

static void dg_checked(long result, int fd, uint32_t phase) {
    if (result < 0) dg_fail(fd, phase, result);
}

static void dg_receive(int fd, uint32_t kind, uint32_t phase) {
    struct dg_message message;
    long result;
    do {
        result = dg_raw(SYS_recvfrom, fd, (long)&message, sizeof(message), MSG_TRUNC, 0, 0);
    } while (result == -EINTR);
    if (result < 0) dg_fail(fd, phase, result);
    if (result != (long)sizeof(message) || message.kind != kind || message.phase != 0 || message.error != 0)
        dg_fail(fd, phase, -EPROTO);
}

static __attribute__((noreturn)) void dg_child(const struct dg_request *request) {
    int error_fd = request->fds[4];
    for (int target = 0; target < 5; ++target) {
        long result = dg_raw(SYS_dup3, request->fds[target], target,
            target >= 3 ? O_CLOEXEC : 0, 0, 0, 0);
        dg_checked(result, error_fd, 1);
        if (target == 4) error_fd = 4;
    }
    dg_checked(dg_raw(SYS_close_range, 5, UINT32_MAX, 0, 0, 0, 0), 4, 2);
    const struct dg_kernel_action action = {0, 0, 0, 0};
    for (int signal = 1; signal <= 64; ++signal) {
        if (signal == SIGKILL || signal == SIGSTOP) continue;
        dg_checked(dg_raw(SYS_rt_sigaction, signal, (long)&action, 0, 8, 0, 0), 4, 3);
    }
    const uint64_t empty_mask = 0;
    dg_checked(dg_raw(SYS_rt_sigprocmask, SIG_SETMASK, (long)&empty_mask, 0, 8, 0, 0), 4, 4);
    /* Init 前 parent 已实际拥有原子 pidfd；阻塞只在 child，parent 可原句柄终止。 */
    dg_receive(4, 3, 5);
    long pid = dg_raw(SYS_getpid, 0, 0, 0, 0, 0, 0);
    dg_checked(pid, 4, 6);
    long session = dg_raw(SYS_setsid, 0, 0, 0, 0, 0, 0);
    dg_checked(session, 4, 6);
    if (session != pid) dg_fail(4, 6, -EPERM);
    /* 先保留实际负errno，成功值才参与身份比较；保留group在session查询前的短路。 */
    long group = dg_raw(SYS_getpgid, 0, 0, 0, 0, 0, 0);
    dg_checked(group, 4, 6);
    if (group != pid) dg_fail(4, 6, -EPERM);
    session = dg_raw(SYS_getsid, 0, 0, 0, 0, 0, 0);
    dg_checked(session, 4, 6);
    if (session != pid) dg_fail(4, 6, -EPERM);
    struct sock_fprog program = {request->filter_count, (struct sock_filter *)request->filter};
    dg_checked(dg_raw(SYS_prctl, PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0, 0), 4, 7);
    dg_checked(dg_raw(SYS_prctl, PR_SET_SECCOMP, 2, (long)&program, 0, 0, 0), 4, 8);
    const struct dg_message ready = {1, 0, 0};
    long sent = dg_raw(SYS_sendto, 4, (long)&ready, sizeof(ready), MSG_NOSIGNAL | MSG_DONTWAIT, 0, 0);
    dg_checked(sent, 4, 9);
    if (sent != (long)sizeof(ready)) dg_fail(4, 9, -EIO);
    dg_receive(4, 4, 10);
    const char empty_path[1] = {0};
    long result = dg_raw(SYS_execveat, 3, (long)empty_path, (long)request->argv,
        (long)request->envp, AT_EMPTY_PATH, 0);
    dg_fail(4, 11, result);
}

/* child 永不返回本函数，更不返回调用它的 Rust；parent 暂保 mask，先构造唯一 owner。 */
long dg_linux_atomic_spawn(const struct dg_request *request, int *pidfd,
        uint64_t *old_mask, int *mask_active) {
    const uint64_t blocked = UINT64_MAX;
    long result = dg_raw(SYS_rt_sigprocmask, SIG_SETMASK, (long)&blocked, (long)old_mask, 8, 0, 0);
    if (result < 0) return result;
    *mask_active = 1;
    result = dg_raw(SYS_clone, CLONE_PIDFD | SIGCHLD, 0, (long)pidfd, 0, 0, 0);
    if (result == 0) dg_child(request);
    return result;
}

long dg_linux_atomic_restore_mask(uint64_t mask) {
    return dg_raw(SYS_rt_sigprocmask, SIG_SETMASK, (long)&mask, 0, 8, 0, 0);
}
