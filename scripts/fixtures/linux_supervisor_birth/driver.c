#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <grp.h>
#include <linux/filter.h>
#include <linux/seccomp.h>
#include <stddef.h>
#include <sys/prctl.h>
#include <sys/syscall.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/wait.h>
#include <sys/xattr.h>
#include <unistd.h>

struct base_request {
    int fds[5]; char *const *argv; char *const *envp;
    unsigned short filter_count; const struct sock_filter *filter;
};
#include "../../../crates/diskgraph-engine/src/native_child/linux_supervisor_spawn.h"
#include "../../../crates/diskgraph-engine/src/native_child/linux_executor_privacy.h"
#ifdef BASELINE
extern long dg_linux_atomic_spawn(const struct base_request *, int *, uint64_t *, int *);
#else

#endif
extern long dg_linux_atomic_restore_mask(uint64_t);
struct message { uint32_t kind, phase; int32_t error; };
static int high(int fd) { int result = fcntl(fd, F_DUPFD_CLOEXEC, 10); if (result < 10) abort(); return result; }
static void pair(int type, int *fds) { if (socketpair(AF_UNIX, type | SOCK_CLOEXEC, 0, fds)) abort(); }
static int receive(int fd, uint32_t kind) {
    struct message message = {0};
    ssize_t size = recv(fd, &message, sizeof(message), 0);
    if (size != (ssize_t)sizeof(message) || message.kind != kind || message.phase || message.error) {
        fprintf(stderr, "startup size=%zd kind=%u phase=%u error=%d\n", size, message.kind, message.phase, message.error);
        return 0;
    }
    return 1;
}
static int send_kind(int fd, uint32_t kind) {
    struct message message = {kind, 0, 0};
    return send(fd, &message, sizeof(message), MSG_NOSIGNAL) == (ssize_t)sizeof(message);
}
static int same_uid_observer(pid_t original, const char *phase) {
    fflush(stdout);
    pid_t observer = fork();
    if (observer < 0) return 0;
    if (observer == 0) {
        if (syscall(SYS_close_range, 3, UINT32_MAX, 0) || setgroups(0, NULL) ||
            setresgid(23111, 23111, 23111) || setresuid(23111, 23111, 23111)) _exit(46);
        char path[64];
        int length = snprintf(path, sizeof(path), "/proc/%d/environ", original);
        if (length <= 0 || length >= (int)sizeof(path)) _exit(46);
        int environment = open(path, O_RDONLY | O_CLOEXEC);
        int rejected = environment < 0 && errno == EACCES;
        printf("{\"same_uid_environ_denied\":%s,\"phase\":\"%s\",\"errno\":%d}\n",
            rejected ? "true" : "false", phase, environment < 0 ? errno : 0);
        if (environment >= 0) close(environment);
        fflush(stdout);
        _exit(rejected ? 0 : 47);
    }
    int status = 0;
    return waitpid(observer, &status, 0) == observer && WIFEXITED(status) && WEXITSTATUS(status) == 0;
}

int main(int argc, char **argv) {
    if (argc != 3 && argc != 4) return 40;
    if (!strcmp(argv[2], "policy_classification")) {
        int passed = dg_executor_protected_mode(0) && dg_executor_protected_mode(2) &&
            !dg_executor_protected_mode(1) && !dg_executor_protected_mode(-1) && !dg_executor_protected_mode(3);
        printf("{\"protected_policy_classification\":%s}\n", passed ? "true" : "false");
        return passed ? 0 : 42;
    }
    int executor_private = !strcmp(argv[2], "executor_private");
    int private_input[2] = {-1, -1};
    if (executor_private && pipe2(private_input, O_CLOEXEC)) return 40;
    int input = executor_private ? private_input[0] : open("/dev/null", O_RDONLY | O_CLOEXEC);
    int output[2], errors[2], startup[2], materials[2], control[2], leak[2];
    if (input < 0 || pipe2(output, O_CLOEXEC) || pipe2(errors, O_CLOEXEC) || pipe2(leak, O_CLOEXEC | O_NONBLOCK)) abort();
    pair(SOCK_SEQPACKET, startup); pair(SOCK_DGRAM, materials); pair(SOCK_STREAM, control);
    int image = open(argv[1], O_RDONLY | O_CLOEXEC | O_NOFOLLOW);
    if (image < 0) abort();
    int source[7] = {input, output[1], errors[1], image, startup[1], materials[1], control[1]};
    struct dg_supervisor_request request = {.uid = 23111, .gid = 23111};
    for (int n = 0; n < 7; ++n) request.fds[n] = high(source[n]);
    if (!strcmp(argv[2], "root_uid")) request.uid = 0;
    if (!strcmp(argv[2], "root_gid")) request.gid = 0;
    if (!strcmp(argv[2], "sentinel_uid")) request.uid = UINT32_MAX;
    if (!strcmp(argv[2], "sentinel_gid")) request.gid = UINT32_MAX;
    if (!strcmp(argv[2], "low_fd")) request.fds[6] = 1;
    if (!strcmp(argv[2], "aliased_fd")) request.fds[6] = request.fds[5];
    uint32_t executor_groups[2] = {23112, 23113};
    int supplementary = !strcmp(argv[2], "supplementary");
    if (supplementary) { request.group_count = 2; request.groups = executor_groups; }
    if (!strcmp(argv[2], "missing_groups")) request.group_count = 1;
    if (!strcmp(argv[2], "oversized_groups")) { request.group_count = 65537; request.groups = executor_groups; }
    if (!strcmp(argv[2], "sentinel_group")) { executor_groups[0] = UINT32_MAX; request.group_count = 1; request.groups = executor_groups; }
    int executor_acl = !strcmp(argv[2], "executor_acl");
    int executor_public = !strcmp(argv[2], "executor_public") || executor_acl;
    int denied_access = !strcmp(argv[2], "denied_access");
    int denied_policy = !strcmp(argv[2], "denied_policy");
    int unprotected_entry = !strcmp(argv[2], "unprotected_entry");
    if (executor_acl) {
        /* 实际POSIX ACL授予客户读取；不能把mode位当作完整凭据判断。 */
        struct acl_entry { uint16_t tag, permissions; uint32_t id; };
        const struct { uint32_t version; struct acl_entry entries[5]; } acl = {2, {
            {1, 5, UINT32_MAX}, {2, 4, 23111}, {4, 1, UINT32_MAX},
            {16, 5, UINT32_MAX}, {32, 1, UINT32_MAX}}};
        if (fsetxattr(image, "system.posix_acl_access", &acl, sizeof(acl), 0)) {
            fprintf(stderr, "fixture ACL setup errno=%d\n", errno); return 43;
        }
        pid_t acl_probe = fork();
        if (acl_probe < 0) return 43;
        if (acl_probe == 0) {
            if (setgroups(0, NULL) || setresgid(23111, 23111, 23111) ||
                setresuid(23111, 23111, 23111)) _exit(44);
            _exit(syscall(SYS_faccessat2, image, "", R_OK, AT_EMPTY_PATH | AT_EACCESS) == 0 ? 0 : 45);
        }
        int acl_status = 0;
        if (waitpid(acl_probe, &acl_status, 0) != acl_probe || !WIFEXITED(acl_status) ||
            WEXITSTATUS(acl_status) != 0) return 43;
        printf("{\"acl_read_permission_verified\":true}\n");
    }
    if (executor_private || executor_public || denied_access || denied_policy) request.executor_protection = 1;
    if (!strcmp(argv[2], "invalid_protection")) request.executor_protection = 2;
    int held_worker = !strcmp(argv[2], "held_worker");
    if (held_worker) {
        if (argc != 4) return 40;
        int worker = open(argv[3], O_RDONLY | O_CLOEXEC | O_NOFOLLOW);
        if (worker < 0) abort();
        request.worker_fd = high(worker); close(worker);
    }
    if (!strcmp(argv[2], "low_worker")) request.worker_fd = 3;
    if (!strcmp(argv[2], "aliased_worker")) request.worker_fd = request.fds[3];
    if (!strcmp(argv[2], "writable_worker")) {
        int writable = open("/dev/null", O_RDWR | O_CLOEXEC);
        if (writable < 0) abort();
        request.worker_fd = high(writable); close(writable);
    }
    char *args[] = {argv[1], supplementary ? "supplementary" : held_worker ? "held_worker" : executor_private ? "private_executor" : unprotected_entry ? "reject_unprotected" : NULL,
        held_worker ? argv[3] : NULL, NULL}; char *environment[] = {NULL};
    request.argv = args; request.envp = environment;
    /* 仅本次隔离driver拒绝setgroups；真实child必须报告降权阶段并保留原pidfd退休。 */
    int denied_groups = !strcmp(argv[2], "denied_groups");
    if (denied_groups || denied_access || denied_policy) {
        const struct sock_filter deny[] = {
            BPF_STMT(BPF_LD | BPF_W | BPF_ABS, offsetof(struct seccomp_data, nr)),
            BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, denied_groups ? SYS_setgroups : denied_policy ? SYS_openat : SYS_faccessat2, 0, 1),
            BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ERRNO | (denied_groups ? EPERM : ENOSYS)),
            BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
        };
        const struct sock_fprog program = {4, (struct sock_filter *)deny};
        if (prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) ||
                prctl(PR_SET_SECCOMP, SECCOMP_MODE_FILTER, &program)) abort();
    }
    int pidfd = -1, active = 0; uint64_t mask = 0;
#ifdef BASELINE
    const struct sock_filter filter[] = {{BPF_RET | BPF_K, 0, 0, 0x7fff0000}};
    struct base_request old = {.argv = args, .envp = environment, .filter_count = 1, .filter = filter};
    for (int n = 0; n < 5; ++n) old.fds[n] = request.fds[n];
    long pid = dg_linux_atomic_spawn(&old, &pidfd, &mask, &active);
#else
    long pid = dg_linux_supervisor_spawn(&request, &pidfd, &mask, &active);
#endif
    if (active && dg_linux_atomic_restore_mask(mask) != 0) abort();
    if (pid < 0) {
        printf("{\"before_birth_rejected\":true,\"errno\":%ld,\"pidfd_unchanged\":%s}\n", -pid, pidfd == -1 ? "true" : "false");
        return !strcmp(argv[2], "clean") ? 41 : 0;
    }
    for (int n = 0; n < 7; ++n) if (n != 6 || request.fds[6] != request.fds[5]) close(request.fds[n]);
    if (request.worker_fd >= 10) close(request.worker_fd);
    for (int n = 0; n < 7; ++n) close(source[n]);
    close(leak[1]);
    /* 原child停在Init之前；公开FD必须已关闭，不以exec完成或子退出代替。 */
    int public_eof = 0;
    for (int n = 0; n < 1000; ++n) {
        char byte; ssize_t size = read(leak[0], &byte, 1);
        if (size == 0) { public_eof = 1; break; }
        if (size != -1 || errno != EAGAIN) break;
        usleep(1000);
    }
    struct message terminal = {0};
    ssize_t terminal_size = -1;
    int handshake;
    int signal_after_ready = !strcmp(argv[2], "signal_after_ready");
    int signalled = -1;
    if (denied_groups || denied_access || executor_public) {
        if (!send_kind(startup[0], 3)) abort();
        terminal_size = recv(startup[0], &terminal, sizeof(terminal), 0);
        handshake = 0;
        if (executor_public && terminal.kind == 1 && !send_kind(startup[0], 4)) abort();
    } else if (signal_after_ready) {
        /* Ready已经在实际降权之后：原pidfd发信号需要CAP_KILL，不能靠Root数值身份冒充。 */
        if (!send_kind(startup[0], 3) || !receive(startup[0], 1)) abort();
        signalled = (int)syscall(SYS_pidfd_send_signal, pidfd, SIGKILL, NULL, 0);
        int signal_error = signalled == 0 ? 0 : errno;
        printf("{\"original_pidfd_signal_sent\":%s,\"signal_errno\":%d}\n",
            signalled == 0 ? "true" : "false", signal_error);
        /* 拒绝信号的负向夹具仍推进原ACK让原child自然退出，绝不丢弃唯一owner。 */
        handshake = signalled == 0 ? 0 : send_kind(startup[0], 4);
    } else {
        handshake = send_kind(startup[0], 3) && receive(startup[0], 1) && send_kind(startup[0], 4);
    }
    int privacy_confirmed = !executor_private;
    if (executor_private) {
        int before = -1, after = -1;
        int observed_before = handshake && read(output[0], &before, sizeof(before)) == (ssize_t)sizeof(before);
        int first_denied = observed_before && dg_executor_protected_mode(before) && same_uid_observer((pid_t)pid, "before_seal");
        char command = 'C';
        if (observed_before && write(private_input[1], &command, 1) != 1) abort();
        int observed_after = observed_before && read(output[0], &after, sizeof(after)) == (ssize_t)sizeof(after);
        int second_denied = observed_after && after == 0 && same_uid_observer((pid_t)pid, "after_seal");
        command = 'D';
        if (observed_after && write(private_input[1], &command, 1) != 1) abort();
        close(private_input[1]);
        privacy_confirmed = first_denied && second_denied;
    }
    char buffer[4096]; ssize_t size = read(output[0], buffer, sizeof(buffer) - 1);
    if (size > 0) { buffer[size] = 0; fputs(buffer, stdout); }
    siginfo_t info = {0};
    int waited = waitid(P_PIDFD, pidfd, &info, WEXITED) == 0;
    if (!denied_groups && !denied_access && !executor_public) terminal_size = recv(startup[0], &terminal, sizeof(terminal), MSG_DONTWAIT);
    printf("{\"startup_after_ack_bytes\":%zd,\"startup_kind\":%u,\"startup_phase\":%u,\"startup_errno\":%d}\n", terminal_size, terminal.kind, terminal.phase, terminal.error);
    printf("{\"original_pidfd_waited\":%s,\"public_eof_before_init\":%s,\"handshake\":%s,\"exited_normally\":%s,\"exit_code\":%d}\n",
        waited ? "true" : "false", public_eof ? "true" : "false", handshake ? "true" : "false", info.si_code == CLD_EXITED ? "true" : "false", info.si_status);
    close(pidfd);
    if (denied_access) return waited && public_eof && terminal.kind == 2 &&
        terminal.phase == 19 && terminal.error == ENOSYS && info.si_code == CLD_EXITED &&
        info.si_status == 127 ? 0 : 42;
    if (executor_public) return waited && public_eof && terminal.kind == 2 &&
        terminal.phase == 19 && terminal.error == EACCES && info.si_code == CLD_EXITED &&
        info.si_status == 127 ? 0 : 42;
    if (signal_after_ready) return signalled == 0 && waited && public_eof &&
        info.si_code == CLD_KILLED && info.si_status == SIGKILL ? 0 : 42;
    return privacy_confirmed && waited && public_eof && handshake && info.si_code == CLD_EXITED && info.si_status == 0 ? 0 : 42;
}
