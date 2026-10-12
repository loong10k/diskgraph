#define _GNU_SOURCE
#include "linux_supervisor_spawn.h"
#include "linux_executor_privacy.h"
#include "linux_atomic_syscall.h"
#include <errno.h>
#include <fcntl.h>
#include <linux/capability.h>
#include <linux/sched.h>
#include <signal.h>
#include <stddef.h>
#include <sys/prctl.h>
#include <sys/socket.h>
#include <sys/syscall.h>
#include <unistd.h>

struct dg_supervisor_message { uint32_t kind, phase; int32_t error; };
struct dg_supervisor_action { uintptr_t handler, flags, restorer; uint64_t mask; };
_Static_assert(sizeof(void *) == 8, "native64 pointers");
_Static_assert(sizeof(struct dg_supervisor_action) == 32, "kernel rt_sigaction layout");
_Static_assert(sizeof(struct dg_supervisor_message) == 12, "fixed startup message");
_Static_assert(offsetof(struct dg_supervisor_request, argv) == 40, "supervisor pointer offset");
_Static_assert(offsetof(struct dg_supervisor_request, group_count) == 56, "supervisor group count offset");
_Static_assert(offsetof(struct dg_supervisor_request, groups) == 64, "supervisor groups offset");
_Static_assert(offsetof(struct dg_supervisor_request, worker_fd) == 72, "supervisor worker offset");
_Static_assert(offsetof(struct dg_supervisor_request, executor_protection) == 76, "supervisor protection offset");
_Static_assert(sizeof(struct dg_supervisor_request) == 80, "supervisor request native64 layout");

static __attribute__((noreturn)) void supervisor_exit(void) {
    dg_raw(SYS_exit_group, 127, 0, 0, 0, 0, 0);
    for (;;) { /* 禁止返回root宿主或运行继承的运行时析构。 */ }
}
static __attribute__((noreturn)) void supervisor_fail(int fd, uint32_t phase, long result) {
    const struct dg_supervisor_message message = {2, phase, (int32_t)-result};
    dg_raw(SYS_sendto, fd, (long)&message, sizeof(message), MSG_NOSIGNAL | MSG_DONTWAIT, 0, 0);
    supervisor_exit();
}
static void supervisor_checked(long result, int fd, uint32_t phase) {
    if (result < 0) supervisor_fail(fd, phase, result);
}
static void supervisor_receive(int fd, uint32_t kind, uint32_t phase) {
    struct dg_supervisor_message message;
    long result;
    do { result = dg_raw(SYS_recvfrom, fd, (long)&message, sizeof(message), MSG_TRUNC, 0, 0); }
    while (result == -EINTR);
    if (result < 0) supervisor_fail(fd, phase, result);
    if (result != (long)sizeof(message) || message.kind != kind || message.phase || message.error)
        supervisor_fail(fd, phase, -EPROTO);
}
static __attribute__((noreturn)) void supervisor_child(const struct dg_supervisor_request *request) {
    int error_fd = request->fds[4];
    for (int target = 0; target < 7; ++target) {
        /* 3/4只用于exec前；5/6是原已准备双私有端点，exec后由角色立即恢复CLOEXEC。 */
        long result = dg_raw(SYS_dup3, request->fds[target], target,
            target == 3 || target == 4 ? O_CLOEXEC : 0, 0, 0, 0);
        supervisor_checked(result, error_fd, 1);
        if (target == 4) error_fd = 4;
    }
    if (request->worker_fd)
        supervisor_checked(dg_raw(SYS_dup3, request->worker_fd, 7, 0, 0, 0, 0), 4, 1);
    supervisor_checked(dg_raw(SYS_close_range, request->worker_fd ? 8 : 7, UINT32_MAX, 0, 0, 0, 0), 4, 2);
    const struct dg_supervisor_action action = {0, 0, 0, 0};
    for (int signal = 1; signal <= 64; ++signal) {
        if (signal == SIGKILL || signal == SIGSTOP) continue;
        supervisor_checked(dg_raw(SYS_rt_sigaction, signal, (long)&action, 0, 8, 0, 0), 4, 3);
    }
    const uint64_t empty_mask = 0;
    supervisor_checked(dg_raw(SYS_rt_sigprocmask, SIG_SETMASK, (long)&empty_mask, 0, 8, 0, 0), 4, 4);
    supervisor_receive(4, 3, 5);
    long pid = dg_raw(SYS_getpid, 0, 0, 0, 0, 0, 0);
    supervisor_checked(pid, 4, 6);
    long session = dg_raw(SYS_setsid, 0, 0, 0, 0, 0, 0);
    supervisor_checked(session, 4, 6);
    if (session != pid) supervisor_fail(4, 6, -EPERM);
    long group = dg_raw(SYS_getpgid, 0, 0, 0, 0, 0, 0);
    supervisor_checked(group, 4, 6);
    if (group != pid) supervisor_fail(4, 6, -EPERM);
    /* root身份不得带入角色：禁保留caps/ambient，并禁止exec重新取得setid或file capabilities。 */
    supervisor_checked(dg_raw(SYS_prctl, PR_SET_KEEPCAPS, 0, 0, 0, 0, 0), 4, 12);
    supervisor_checked(dg_raw(SYS_prctl, PR_CAP_AMBIENT, PR_CAP_AMBIENT_CLEAR_ALL, 0, 0, 0, 0), 4, 13);
    supervisor_checked(dg_raw(SYS_prctl, PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0, 0), 4, 14);
    supervisor_checked(dg_raw(SYS_setgroups, request->group_count, (long)request->groups, 0, 0, 0, 0), 4, 15);
    supervisor_checked(dg_raw(SYS_setresgid, request->gid, request->gid, request->gid, 0, 0, 0), 4, 16);
    supervisor_checked(dg_raw(SYS_setresuid, request->uid, request->uid, request->uid, 0, 0, 0), 4, 17);
    const struct __user_cap_header_struct capabilities_header = {_LINUX_CAPABILITY_VERSION_3, 0};
    const struct __user_cap_data_struct capabilities[2] = {{0, 0, 0}, {0, 0, 0}};
    supervisor_checked(dg_raw(SYS_capset, (long)&capabilities_header, (long)&capabilities, 0, 0, 0, 0), 4, 18);
    if (request->executor_protection) {
        /* 使用已采用的真实客户凭据核验原ELF，包含辅助组/ACL/LSM；未知拒绝原因不降级。 */
        const char image_path[1] = {0};
        long readable = dg_raw(SYS_faccessat2, 3, (long)image_path, R_OK, AT_EMPTY_PATH | AT_EACCESS, 0, 0);
        if (readable != -EACCES) supervisor_fail(4, 19, readable < 0 ? readable : -EACCES);
        /* 保护ACK等待期间的原环境；exec后的隐私仍需目标内核独立验收。 */
        supervisor_checked(dg_raw(SYS_prctl, PR_SET_DUMPABLE, 0, 0, 0, 0, 0), 4, 20);
    }
    const struct dg_supervisor_message ready = {1, 0, 0};
    long sent = dg_raw(SYS_sendto, 4, (long)&ready, sizeof(ready), MSG_NOSIGNAL | MSG_DONTWAIT, 0, 0);
    supervisor_checked(sent, 4, 9);
    if (sent != (long)sizeof(ready)) supervisor_fail(4, 9, -EIO);
    supervisor_receive(4, 4, 10);
    const char path[1] = {0};
    long result = dg_raw(SYS_execveat, 3, (long)path, (long)request->argv, (long)request->envp, AT_EMPTY_PATH, 0);
    supervisor_fail(4, 11, result);
}

/* 所有指针与源对象是受信宿主出生前材料；不分配、不回调、不在child调用libc或Rust。 */
long dg_linux_supervisor_spawn(const struct dg_supervisor_request *request,
        int *pidfd, uint64_t *old_mask, int *mask_active) {
    if (dg_raw(SYS_getuid, 0, 0, 0, 0, 0, 0) != 0 || dg_raw(SYS_geteuid, 0, 0, 0, 0, 0, 0) != 0 ||
        dg_raw(SYS_getgid, 0, 0, 0, 0, 0, 0) != 0 || dg_raw(SYS_getegid, 0, 0, 0, 0, 0, 0) != 0)
        return -EPERM;
    if (!request->uid || !request->gid || request->uid == UINT32_MAX || request->gid == UINT32_MAX)
        return -EPERM;
    /* Linux NGROUPS_MAX；原数组由父侧持有直至clone完成，禁止child分配或读取客户字段。 */
    if (request->executor_protection > 1) return -EINVAL;
    if (request->group_count > 65536 || (request->group_count && !request->groups)) return -EINVAL;
    for (uint32_t index = 0; index < request->group_count; ++index)
        if (request->groups[index] == UINT32_MAX) return -EINVAL;
    for (int index = 0; index < 7; ++index) {
        if (request->fds[index] < 10) return -EINVAL;
        for (int previous = 0; previous < index; ++previous)
            if (request->fds[index] == request->fds[previous]) return -EINVAL;
    }
    if (request->worker_fd) {
        if (request->worker_fd < 10) return -EINVAL;
        for (int index = 0; index < 7; ++index)
            if (request->worker_fd == request->fds[index]) return -EINVAL;
        long flags = dg_raw(SYS_fcntl, request->worker_fd, F_GETFL, 0, 0, 0, 0);
        if (flags < 0) return flags;
        if ((flags & O_ACCMODE) != O_RDONLY || (flags & O_PATH)) return -EINVAL;
    }
    if (request->executor_protection) {
        long policy = dg_executor_kernel_policy();
        if (policy < 0) return policy;
    }
    const uint64_t blocked = UINT64_MAX;
    long result = dg_raw(SYS_rt_sigprocmask, SIG_SETMASK, (long)&blocked, (long)old_mask, 8, 0, 0);
    if (result < 0) return result;
    *mask_active = 1;
    result = dg_raw(SYS_clone, CLONE_PIDFD | SIGCHLD, 0, (long)pidfd, 0, 0, 0);
    if (result == 0) supervisor_child(request);
    return result;
}

/* 供受信executor原入口调用，与原生角色验收共用真实内核操作。 */
long dg_linux_executor_seal_privacy(void) { return dg_executor_seal_privacy(); }
long dg_linux_executor_kernel_policy(void) { return dg_executor_kernel_policy(); }
