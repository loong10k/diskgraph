#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
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
#include <unistd.h>

struct base_request {
    int fds[5]; char *const *argv; char *const *envp;
    unsigned short filter_count; const struct sock_filter *filter;
};
#include "../../../crates/diskgraph-engine/src/native_child/linux_supervisor_spawn.h"
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
int main(int argc, char **argv) {
    if (argc != 3) return 40;
    int input = open("/dev/null", O_RDONLY | O_CLOEXEC);
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
    char *args[] = {argv[1], supplementary ? "supplementary" : NULL, NULL}; char *environment[] = {NULL};
    request.argv = args; request.envp = environment;
    /* 仅本次隔离driver拒绝setgroups；真实child必须报告降权阶段并保留原pidfd退休。 */
    int denied_groups = !strcmp(argv[2], "denied_groups");
    if (denied_groups) {
        const struct sock_filter deny[] = {
            BPF_STMT(BPF_LD | BPF_W | BPF_ABS, offsetof(struct seccomp_data, nr)),
            BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, SYS_setgroups, 0, 1),
            BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ERRNO | EPERM),
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
    if (denied_groups) {
        if (!send_kind(startup[0], 3)) abort();
        terminal_size = recv(startup[0], &terminal, sizeof(terminal), 0);
        handshake = 0;
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
    char buffer[4096]; ssize_t size = read(output[0], buffer, sizeof(buffer) - 1);
    if (size > 0) { buffer[size] = 0; fputs(buffer, stdout); }
    siginfo_t info = {0};
    int waited = waitid(P_PIDFD, pidfd, &info, WEXITED) == 0;
    if (!denied_groups) terminal_size = recv(startup[0], &terminal, sizeof(terminal), MSG_DONTWAIT);
    printf("{\"startup_after_ack_bytes\":%zd,\"startup_kind\":%u,\"startup_phase\":%u,\"startup_errno\":%d}\n", terminal_size, terminal.kind, terminal.phase, terminal.error);
    printf("{\"original_pidfd_waited\":%s,\"public_eof_before_init\":%s,\"handshake\":%s,\"exited_normally\":%s,\"exit_code\":%d}\n",
        waited ? "true" : "false", public_eof ? "true" : "false", handshake ? "true" : "false", info.si_code == CLD_EXITED ? "true" : "false", info.si_status);
    close(pidfd);
    if (signal_after_ready) return signalled == 0 && waited && public_eof &&
        info.si_code == CLD_KILLED && info.si_status == SIGKILL ? 0 : 42;
    return waited && public_eof && handshake && info.si_code == CLD_EXITED && info.si_status == 0 ? 0 : 42;
}
