#define _GNU_SOURCE
/* 真实 Linux syscall/pthread 动作夹具；目标过滤器只由 Rust 生产入口安装。 */
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <linux/filter.h>
#include <linux/seccomp.h>
#include <pthread.h>
#include <sched.h>
#include <signal.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/prctl.h>
#include <sys/syscall.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

static const char *directory;
static long thread_tid;
static long thread_tgid;
static int thread_fork_errno;

static void fatal(const char *stage) {
    fprintf(stderr, "scanner sandbox fixture failed: %s errno=%d\n", stage, errno);
    _exit(2);
}

static void path_at(char *path, size_t size, const char *name) {
    int length = snprintf(path, size, "%s/%s", directory, name);
    if (length < 0 || (size_t)length >= size) { errno = ENAMETOOLONG; fatal("path"); }
}

static void scalar(const char *name, long value) {
    char path[PATH_MAX];
    path_at(path, sizeof(path), name);
    FILE *file = fopen(path, "w");
    if (!file || fprintf(file, "%ld", value) < 0 || fclose(file) != 0) { fatal("scalar"); }
}

static void identity(void) {
    scalar("pid", getpid());
    scalar("main_tid", syscall(SYS_gettid));
    scalar("pgid", getpgrp());
    scalar("sid", getsid(0));
    scalar("ppid", getppid());
    scalar("no_new_privs", prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0));
    scalar("seccomp", prctl(PR_GET_SECCOMP, 0, 0, 0, 0));
    scalar("ready", 1);
}

static int record_child_result(long result) {
    int error = errno;
    if (result == 0) { _exit(91); }
    if (result > 0) {
        int status;
        while (waitpid((pid_t)result, &status, 0) < 0) {
            if (errno != EINTR) { fatal("unexpected child reap"); }
        }
        return 0;
    }
    return error;
}

static void *thread_check(void *unused) {
    (void)unused;
    thread_tid = syscall(SYS_gettid);
    thread_tgid = getpid();
    errno = 0;
    thread_fork_errno = record_child_result(fork());
    return NULL;
}

static int unexpected_clone(void *unused) {
    (void)unused;
    return 91;
}

static int clone_attempt(int flags) {
    const size_t size = 128 * 1024;
    void *stack = malloc(size);
    if (!stack) { fatal("clone stack"); }
    errno = 0;
    int result = clone(unexpected_clone, (char *)stack + size, flags, NULL);
    int error = errno;
    /* 所有测试 flags 均非有效线程组合；若过滤器缺失，普通 child 也自然回收。 */
    if (result > 0) { (void)record_child_result(result); }
    free(stack);
    return result < 0 ? error : 0;
}

static int error_of(long result) { return result < 0 ? errno : 0; }

static void report(void) {
    identity();
    pthread_t thread;
    int created = pthread_create(&thread, NULL, thread_check, NULL);
    if (created != 0) { errno = created; fatal("pthread create"); }
    int joined = pthread_join(thread, NULL);
    if (joined != 0) { errno = joined; fatal("pthread join"); }
    errno = 0;
    int fork_error = record_child_result(fork());
    errno = 0;
    long vfork_result = vfork();
    if (vfork_result == 0) { _exit(91); }
    int vfork_error = record_child_result(vfork_result);
    errno = 0;
    int process_clone = record_child_result(syscall(SYS_clone, SIGCHLD, NULL, NULL, NULL, 0));
    errno = 0;
    int upper_flags = record_child_result(syscall(SYS_clone, (1ULL << 32) | SIGCHLD,
                                                  NULL, NULL, NULL, 0));
    errno = 0;
    int clone3_error = error_of(syscall(SYS_clone3, NULL, 0));
    int no_vm = clone_attempt(CLONE_THREAD | CLONE_SIGHAND);
    int no_sighand = clone_attempt(CLONE_THREAD | CLONE_VM);
    int namespace_flags = clone_attempt(CLONE_THREAD | CLONE_VM | CLONE_SIGHAND | CLONE_NEWUSER);
    errno = 0;
    int unshare_error = error_of(syscall(SYS_unshare, 0));
    errno = 0;
    int setns_error = error_of(syscall(SYS_setns, -1, 0));
    errno = 0;
    int setsid_error = error_of(syscall(SYS_setsid));
    errno = 0;
    int setpgid_error = error_of(syscall(SYS_setpgid, 0, 0));
    errno = 0;
    int ptrace_error = error_of(syscall(SYS_ptrace, 0, 0, NULL, NULL));
    errno = 0;
    int uring_setup = error_of(syscall(SYS_io_uring_setup, 0, NULL));
    errno = 0;
    int uring_enter = error_of(syscall(SYS_io_uring_enter, -1, 0, 0, 0, NULL, 0));
    errno = 0;
    int uring_register = error_of(syscall(SYS_io_uring_register, -1, 0, NULL, 0));
    printf("{\"pthread_create\":%d,\"pthread_join\":%d,\"thread_tid\":%ld,"
           "\"thread_tgid\":%ld,\"thread_fork_errno\":%d,\"fork_errno\":%d,"
           "\"vfork_errno\":%d,\"process_clone_errno\":%d,\"upper_flags_errno\":%d,"
           "\"clone3_errno\":%d,\"no_vm_errno\":%d,\"no_sighand_errno\":%d,"
           "\"namespace_flags_errno\":%d,\"unshare_errno\":%d,\"setns_errno\":%d,"
           "\"setsid_errno\":%d,\"setpgid_errno\":%d,\"ptrace_errno\":%d,"
           "\"uring_setup_errno\":%d,\"uring_enter_errno\":%d,\"uring_register_errno\":%d}\n",
           created, joined, thread_tid, thread_tgid, thread_fork_errno, fork_error,
           vfork_error, process_clone, upper_flags, clone3_error, no_vm, no_sighand,
           namespace_flags, unshare_error, setns_error, setsid_error, setpgid_error,
           ptrace_error, uring_setup, uring_enter, uring_register);
    if (fflush(stdout) != 0) { fatal("report flush"); }
}

struct heartbeat_argument { unsigned index; };

static void *heartbeat(void *opaque) {
    unsigned index = ((const struct heartbeat_argument *)opaque)->index;
    char release[PATH_MAX], temporary[PATH_MAX], target[PATH_MAX], name[64];
    path_at(release, sizeof(release), "release");
    snprintf(name, sizeof(name), "heartbeat_%u_next", index);
    path_at(temporary, sizeof(temporary), name);
    snprintf(name, sizeof(name), "heartbeat_%u", index);
    path_at(target, sizeof(target), name);
    snprintf(name, sizeof(name), "tid_%u", index);
    scalar(name, syscall(SYS_gettid));
    struct timespec pause = { .tv_sec = 0, .tv_nsec = 1000000 };
    for (long sequence = 1; access(release, F_OK) != 0; ++sequence) {
        FILE *file = fopen(temporary, "w");
        if (!file || fprintf(file, "%ld", sequence) < 0 || fclose(file) != 0 ||
            rename(temporary, target) != 0) { fatal("heartbeat"); }
        if (sequence > 60000) { errno = ETIMEDOUT; fatal("heartbeat watchdog"); }
        nanosleep(&pause, NULL);
    }
    snprintf(name, sizeof(name), "finished_%u", index);
    scalar(name, 1);
    return NULL;
}

static void threads_after_main(void) {
    identity();
    struct heartbeat_argument *arguments = calloc(2, sizeof(*arguments));
    if (!arguments) { fatal("thread arguments"); }
    pthread_t threads[2];
    for (unsigned index = 0; index < 2; ++index) {
        arguments[index].index = index;
        int error = pthread_create(&threads[index], NULL, heartbeat, &arguments[index]);
        if (error != 0) { errno = error; fatal("heartbeat pthread create"); }
    }
    scalar("main_leaving", 1);
    /* 原 main 只退出自身，两个 pthread 保留真实 heartbeat；管道共享且全部关闭。 */
    if (close(STDOUT_FILENO) != 0 || close(STDERR_FILENO) != 0) { fatal("close stdio"); }
    pthread_exit(NULL);
}

static void deny_install_and_exec(const char *host) {
    /* 仅测试失败宿主：外层过滤器拒安装内层过滤器，不复制目标派生算法。 */
    struct sock_filter filter[] = {
        BPF_STMT(BPF_LD | BPF_W | BPF_ABS, offsetof(struct seccomp_data, nr)),
        BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, SYS_seccomp, 4, 0),
        BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, SYS_prctl, 0, 2),
        BPF_STMT(BPF_LD | BPF_W | BPF_ABS, offsetof(struct seccomp_data, args[0])),
        BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, PR_SET_SECCOMP, 1, 0),
        BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
        BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ERRNO | EPERM),
    };
    struct sock_fprog program = { .len = sizeof(filter) / sizeof(filter[0]), .filter = filter };
    if (prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 ||
        prctl(PR_SET_SECCOMP, SECCOMP_MODE_FILTER, &program) != 0) { fatal("outer test filter"); }
    if (setenv("DG_SCANNER_FILTER_DENIED_HOST", directory, 1) != 0) { fatal("host environment"); }
    execl(host, host, "--exact", "native_child::linux_scanner_sandbox_fixture::filter_denied_host",
          "--nocapture", (char *)NULL);
    fatal("exec isolated host");
}

int main(int argc, char **argv) {
    if (argc < 3) { errno = EINVAL; fatal("arguments"); }
    directory = argv[2];
    if (strcmp(argv[1], "exit") == 0) { identity(); return 0; }
    if (strcmp(argv[1], "report") == 0) { report(); return 0; }
    if (strcmp(argv[1], "threads") == 0) { threads_after_main(); return 3; }
    if (strcmp(argv[1], "deny_install") == 0 && argc == 4) { deny_install_and_exec(argv[3]); }
#if defined(__x86_64__)
    if (strcmp(argv[1], "x32") == 0) {
        identity();
        scalar("about_to_call", 1);
        (void)syscall(0x40000000UL | SYS_getpid);
        scalar("after_wrong_abi", 1);
        return 3;
    }
    if (strcmp(argv[1], "compat") == 0) {
        identity();
        scalar("about_to_call", 1);
        long result;
        __asm__ volatile("int $0x80" : "=a"(result) : "a"(20) : "memory");
        scalar("after_wrong_abi", result);
        return 3;
    }
#endif
    errno = EINVAL;
    fatal("unknown mode");
}
