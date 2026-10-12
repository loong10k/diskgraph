#ifndef DG_LINUX_EXECUTOR_PRIVACY_H
#define DG_LINUX_EXECUTOR_PRIVACY_H
#include "linux_atomic_syscall.h"
#include <errno.h>
#include <fcntl.h>
#include <sys/prctl.h>
#include <sys/syscall.h>

/* 0与suidsafe 2均禁止无能力的同UID查看；调试模式1必须在出生前拒绝。 */
static inline int dg_executor_protected_mode(long mode) { return mode == 0 || mode == 2; }

/* 只读固定内核策略，固定4字节缓冲；不修改sysctl，不读取客户路径或模拟安全状态。 */
static inline long dg_executor_kernel_policy(void) {
    const char path[] = "/proc/sys/fs/suid_dumpable";
    long fd = dg_raw(SYS_openat, AT_FDCWD, (long)path, O_RDONLY | O_CLOEXEC | O_NOFOLLOW, 0, 0, 0);
    if (fd < 0) return fd;
    char policy[4] = {0, 0, 0, 0};
    long count = dg_raw(SYS_read, fd, (long)policy, sizeof(policy), 0, 0, 0);
    long closed = dg_raw(SYS_close, fd, 0, 0, 0, 0, 0);
    if (count < 0) return count;
    if (closed < 0) return closed;
    if (count != 2 || policy[1] != '\n' || !dg_executor_protected_mode(policy[0] - '0')) return -EPERM;
    return 0;
}

/* 只从原受保护状态收敛到0；拒绝Root或已暴露的1，不给予身份/会话/镜像信任。 */
static inline long dg_executor_seal_privacy(void) {
    long uid = dg_raw(SYS_getuid, 0, 0, 0, 0, 0, 0);
    long effective = dg_raw(SYS_geteuid, 0, 0, 0, 0, 0, 0);
    if (uid <= 0 || effective != uid) return -EPERM;
    long before = dg_raw(SYS_prctl, PR_GET_DUMPABLE, 0, 0, 0, 0, 0);
    if (before < 0) return before;
    if (!dg_executor_protected_mode(before)) return -EPERM;
    long sealed = dg_raw(SYS_prctl, PR_SET_DUMPABLE, 0, 0, 0, 0, 0);
    if (sealed < 0) return sealed;
    long after = dg_raw(SYS_prctl, PR_GET_DUMPABLE, 0, 0, 0, 0, 0);
    if (after < 0) return after;
    return after == 0 ? 0 : -EPERM;
}
#endif
