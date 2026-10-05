/* Linux 原子出生机制探针；不复制 scanner 过滤器，不证明 Rust 生产入口。 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <linux/audit.h>
#include <linux/filter.h>
#include <linux/sched.h>
#include <linux/seccomp.h>
#include <poll.h>
#include <pthread.h>
#include <signal.h>
#include <stdatomic.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/prctl.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

#ifndef DG_ATOMIC_IMAGE_ID
#define DG_ATOMIC_IMAGE_ID 1
#endif
#define K_PIDFD 3
#define K_CLOEXEC 4
#define K_EMPTY_PATH 0x1000
#if defined(__x86_64__) && !defined(__ILP32__)
#define DG_ABI "x86_64"
#define DG_ARCH AUDIT_ARCH_X86_64
#elif defined(__aarch64__) && __BYTE_ORDER__ == __ORDER_LITTLE_ENDIAN__
#define DG_ABI "aarch64"
#define DG_ARCH AUDIT_ARCH_AARCH64
#else
#error "this native qualification requires Linux x86_64 or little-endian aarch64"
#endif

/* 子分支仅走此原生 ABI；不经过 libc errno、atfork、分配器或锁。 */
static long raw6(long nr, long a, long b, long c, long d, long e, long f) {
#if defined(__x86_64__)
    register long r10 __asm__("r10") = d;
    register long r8 __asm__("r8") = e;
    register long r9 __asm__("r9") = f;
    long result;
    __asm__ volatile("syscall" : "=a"(result)
        : "a"(nr), "D"(a), "S"(b), "d"(c), "r"(r10), "r"(r8), "r"(r9)
        : "rcx", "r11", "cc", "memory");
    return result;
#else
    register long x8 __asm__("x8") = nr;
    register long x0 __asm__("x0") = a;
    register long x1 __asm__("x1") = b;
    register long x2 __asm__("x2") = c;
    register long x3 __asm__("x3") = d;
    register long x4 __asm__("x4") = e;
    register long x5 __asm__("x5") = f;
    __asm__ volatile("svc 0" : "+r"(x0)
        : "r"(x8), "r"(x1), "r"(x2), "r"(x3), "r"(x4), "r"(x5) : "cc", "memory");
    return x0;
#endif
}
#define PTR(p) ((long)(intptr_t)(p))
#define RAW(n,a,b,c,d,e,f) raw6((n),(long)(a),(long)(b),(long)(c),(long)(d),(long)(e),(long)(f))

struct owned_child { int pidfd; long pid; int reaped; };
struct record { int phase, error, prepare, parent, child, handler; };
struct external_reaper { int fd; long result; siginfo_t info; };
/* 来源：两种 Linux UAPI 的 64-bit rt_sigaction，不使用 libc sigaction 布局。 */
struct kernel_action { uintptr_t handler, flags, restorer; uint64_t mask; };
_Static_assert(sizeof(struct kernel_action) == 32, "native kernel sigaction ABI");
static volatile sig_atomic_t prepare_calls, parent_calls, child_calls, handler_calls;
static int handler_fd = -1;
static atomic_int thread_ready, release_threads;

static void on_prepare(void) { ++prepare_calls; }
static void on_parent(void) { ++parent_calls; }
static void on_child(void) { ++child_calls; }
static void on_signal(int signal_number) {
    (void)signal_number;
    ++handler_calls;
    if (handler_fd >= 0) (void)RAW(SYS_write, handler_fd, PTR("H"), 1, 0, 0, 0);
}

static long birth(struct owned_child *p) {
#if defined(__x86_64__)
    long result = RAW(SYS_clone, CLONE_PIDFD | SIGCHLD, 0, PTR(&p->pidfd), 0, 0, 0);
#else
    /* ARM64 的第四个参数为 tls，第五个为 child_tid，两者明确为零。 */
    long result = RAW(SYS_clone, CLONE_PIDFD | SIGCHLD, 0, PTR(&p->pidfd), 0, 0, 0);
#endif
    if (result > 0) p->pid = result;
    return result;
}

static long signal_original(const struct owned_child *p, int signal_number) {
    return RAW(SYS_pidfd_send_signal, p->pidfd, signal_number, 0, 0, 0, 0);
}
static int native_ok(long result, int *error) {
    if (result < 0) { *error = (int)-result; return 0; }
    return 1;
}

static long wait_original(struct owned_child *p, siginfo_t *info, int flags) {
    memset(info, 0, sizeof(*info));
    long result;
    do { result = RAW(SYS_waitid, K_PIDFD, p->pidfd, PTR(info), WEXITED | flags, 0, 0); }
    while (result == -EINTR);
    if (result == 0 && info->si_pid != 0 && !(flags & WNOWAIT)) p->reaped = 1;
    return result;
}

/* 所有父路径先真实 stop/wait，再释放原 FD；从不由 PID 重新打开或信号数字组。 */
static int cleanup(struct owned_child *p) {
    int failure = 0;
    if (p->pidfd >= 0 && !p->reaped) {
        long stopped = signal_original(p, SIGKILL);
        if (stopped < 0 && stopped != -ESRCH) failure = (int)-stopped;
        siginfo_t info;
        long waited = wait_original(p, &info, 0);
        if (waited < 0 && !failure) failure = (int)-waited;
    }
    if (p->pidfd >= 0) close(p->pidfd);
    p->pidfd = -1;
    return failure;
}

static void child_error(int output, int phase, long error) __attribute__((noreturn));
static void child_error(int output, int phase, long error) {
    struct record report = {phase, (int)-error, 0, 0, 0, 0};
    (void)RAW(SYS_write, output, PTR(&report), sizeof(report), 0, 0, 0);
    (void)RAW(SYS_exit_group, 127, 0, 0, 0, 0, 0);
    __builtin_unreachable();
}

enum child_mode { HELD, NATURAL, SIGNALLED, IMAGE, CLOSE_DENIED };
static void child_start(enum child_mode mode, int control[2], int output[2],
                        int executable, const char *canary_text, uint64_t original_mask) __attribute__((noreturn));
static void child_start(enum child_mode mode, int control[2], int output[2],
                        int executable, const char *canary_text, uint64_t original_mask) {
    (void)RAW(SYS_close, control[1], 0, 0, 0, 0, 0);
    (void)RAW(SYS_close, output[0], 0, 0, 0, 0, 0);
    if (mode == NATURAL) {
        (void)RAW(SYS_exit_group, 37, 0, 0, 0, 0, 0);
        __builtin_unreachable();
    }
    if (mode == SIGNALLED) {
        struct kernel_action inherited = {0, 0, 0, 0}, reset = {0, 0, 0, 0};
        long queried = RAW(SYS_rt_sigaction, SIGUSR1, 0, PTR(&inherited), 8, 0, 0);
        if (queried < 0 || inherited.handler != (uintptr_t)on_signal)
            child_error(output[1], 3, queried < 0 ? queried : -EINVAL);
        long changed = RAW(SYS_rt_sigaction, SIGUSR1, PTR(&reset), 0, 8, 0, 0);
        if (changed < 0) child_error(output[1], 3, changed);
        struct record report = {3, 0, prepare_calls, parent_calls, child_calls, handler_calls};
        if (RAW(SYS_write, output[1], PTR(&report), sizeof(report), 0, 0, 0) != (long)sizeof(report))
            child_error(output[1], 3, -EIO);
    }
    if (mode == HELD || mode == SIGNALLED) {
        char release;
        long received = RAW(SYS_read, control[0], PTR(&release), 1, 0, 0, 0);
        if (received != 1) child_error(output[1], 4, received < 0 ? received : -EPIPE);
        if (mode == SIGNALLED) {
            long restored = RAW(SYS_rt_sigprocmask, SIG_SETMASK, PTR(&original_mask), 0, 8, 0, 0);
            if (restored < 0) child_error(output[1], 4, restored);
        }
        (void)RAW(SYS_exit_group, 0, 0, 0, 0, 0, 0);
        __builtin_unreachable();
    }
    long closed = RAW(SYS_close_range, 3, UINT32_MAX, K_CLOEXEC, 0, 0, 0);
    if (closed < 0) child_error(output[1], 5, closed);
    long duplicated = RAW(SYS_dup3, output[1], STDOUT_FILENO, 0, 0, 0, 0);
    if (duplicated < 0) child_error(output[1], 6, duplicated);
    char *arguments[] = {(char *)"qualified-held-image", (char *)"image", (char *)"canary", (char *)canary_text, NULL};
    char *environment[] = {NULL};
    long executed = RAW(SYS_execveat, executable, PTR(""), PTR(arguments), PTR(environment), K_EMPTY_PATH, 0);
    child_error(output[1], 7, executed);
}

static int start(struct owned_child *p, enum child_mode mode, int control[2],
                 int output[2], int executable, const char *canary_text, uint64_t original_mask) {
    long result = birth(p);
    if (result == 0) child_start(mode, control, output, executable, canary_text, original_mask);
    if (result < 0) return (int)-result;
    close(control[0]); control[0] = -1;
    close(output[1]); output[1] = -1;
    return 0;
}

/* 独立 fixture 宿主的单 syscall 拒绝，不复刻任何生产 scanner 过滤器。 */
static int deny_syscall(int nr, int error) {
    struct sock_filter instructions[] = {
        BPF_STMT(BPF_LD | BPF_W | BPF_ABS, offsetof(struct seccomp_data, arch)),
        BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, DG_ARCH, 1, 0),
        BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_KILL_PROCESS),
        BPF_STMT(BPF_LD | BPF_W | BPF_ABS, offsetof(struct seccomp_data, nr)),
        BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, (unsigned int)nr, 0, 1),
        BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ERRNO | (unsigned int)error),
        BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW)
    };
    struct sock_fprog program = {7, instructions};
    if (prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) < 0) return errno;
    if (prctl(PR_SET_SECCOMP, SECCOMP_MODE_FILTER, &program) < 0) return errno;
    return 0;
}

static void close_pair(int pair[2]) {
    for (int i = 0; i < 2; ++i) if (pair[i] >= 0) { close(pair[i]); pair[i] = -1; }
}
static int read_bytes(int fd, void *buffer, size_t capacity) {
    size_t used = 0;
    while (used < capacity) {
        ssize_t count = read(fd, (char *)buffer + used, capacity - used);
        if (count == 0) return (int)used;
        if (count < 0) { if (errno == EINTR) continue; return -errno; }
        used += (size_t)count;
    }
    return (int)used;
}
static void *holding_thread(void *argument) {
    pthread_mutex_t *mutex = argument;
    pthread_mutex_lock(mutex);
    atomic_fetch_add(&thread_ready, 1);
    while (!atomic_load(&release_threads)) { struct timespec step = {0, 1000000}; nanosleep(&step, NULL); }
    pthread_mutex_unlock(mutex);
    return NULL;
}
static void *reap_from_other_thread(void *argument) {
    struct external_reaper *reaper = argument;
    memset(&reaper->info, 0, sizeof(reaper->info));
    do { reaper->result = RAW(SYS_waitid, K_PIDFD, reaper->fd, PTR(&reaper->info), WEXITED, 0, 0); }
    while (reaper->result == -EINTR);
    return NULL;
}
static int copy_image(const char *source, const char *destination) {
    int from = open(source, O_RDONLY | O_CLOEXEC), to = -1, error = 0;
    if (from < 0) return errno;
    to = open(destination, O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC, 0700);
    if (to < 0) { error = errno; goto done; }
    char block[4096]; ssize_t count;
    while ((count = read(from, block, sizeof(block))) > 0) {
        ssize_t used = 0;
        while (used < count) { ssize_t written = write(to, block + used, (size_t)(count - used));
            if (written <= 0) { error = errno ? errno : EIO; goto done; } used += written; }
    }
    if (count < 0) error = errno;
done:
    close(from); if (to >= 0) close(to); return error;
}

static int run(const char *mode, const char *directory, const char *self, const char *replacement) {
    struct owned_child first = {-1, 0, 0}, other = {-1, 0, 0};
    int control[2] = {-1, -1}, output[2] = {-1, -1}, second_control[2] = {-1, -1}, second_output[2] = {-1, -1};
    int error = 0, executable = -1, canary = -1, thread_count = 0, handler_installed = 0, mask_changed = 0;
    int actual_error = 0, actual_code = 0, output_bytes = 0, cloexec = 0, other_live = 0;
    int external_reap_observed = 0, ambient_closed = 0, parent_handler_preserved = 0;
    const char *phase = "prepare";
    siginfo_t info = {0}; char bytes[256] = {0}, canary_text[32] = {0}; uint64_t original_mask = 0, blocked = UINT64_MAX;
    pthread_t threads[2]; pthread_mutex_t mutexes[2] = {PTHREAD_MUTEX_INITIALIZER, PTHREAD_MUTEX_INITIALIZER};
    pthread_t reaping_thread; int reaping_started = 0;
    struct external_reaper reaper = {.fd = -1};
    struct sigaction original_action = {0}, action = {0}, original_pipe = {0}, pipe_action = {0};
    int pipe_handler_installed = 0;
#define REQUIRE(condition, label) do { if (!(condition)) { phase = (label); if (!error) error = EINVAL; goto done; } } while (0)
    // 只约束独立 fixture 宿主：异常 peer close 必须返回 EPIPE，不能绕过自创建 child 的清理。
    pipe_action.sa_handler = SIG_IGN;
    if (sigaction(SIGPIPE, &pipe_action, &original_pipe) < 0) { error = errno; goto done; }
    pipe_handler_installed = 1;
    if (pipe2(control, O_CLOEXEC) < 0 || pipe2(output, O_CLOEXEC) < 0) { error = errno; goto done; }
    if (!strcmp(mode, "clone_denied")) {
        error = deny_syscall(SYS_clone, EACCES); REQUIRE(!error, "deny_install");
        long result = birth(&first);
        if (result == 0) { (void)RAW(SYS_exit_group, 127, 0, 0, 0, 0, 0); __builtin_unreachable(); }
        actual_error = (int)-result;
        REQUIRE(result == -EACCES && first.pidfd == -1, "clone_denial");
        REQUIRE(waitpid(-1, NULL, WNOHANG) == -1 && errno == ECHILD, "zero_child");
        goto done;
    }
    if (!strcmp(mode, "clone3_denied")) {
        error = deny_syscall(SYS_clone3, ENOSYS); REQUIRE(!error, "deny_install");
        struct clone_args arguments; memset(&arguments, 0, sizeof(arguments));
        arguments.flags = CLONE_PIDFD; arguments.pidfd = (uint64_t)(uintptr_t)&first.pidfd; arguments.exit_signal = SIGCHLD;
        long result = RAW(SYS_clone3, PTR(&arguments), sizeof(arguments), 0, 0, 0, 0);
        if (result == 0) { (void)RAW(SYS_exit_group, 127, 0, 0, 0, 0, 0); __builtin_unreachable(); }
        if (result > 0) first.pid = result;
        actual_error = (int)-result;
        REQUIRE(result == -ENOSYS && first.pidfd == -1, "clone3_real_enosys");
    }
    if (!strcmp(mode, "signals")) {
        REQUIRE(pthread_atfork(on_prepare, on_parent, on_child) == 0, "register_atfork");
        action.sa_handler = on_signal; sigemptyset(&action.sa_mask);
        REQUIRE(sigaction(SIGUSR1, &action, &original_action) == 0, "register_handler"); handler_installed = 1;
        for (int i = 0; i < 2; ++i) { error = pthread_create(&threads[i], NULL, holding_thread, &mutexes[i]);
            REQUIRE(!error, "pthread_create"); ++thread_count; }
        while (atomic_load(&thread_ready) != 2) { struct timespec step = {0, 1000000}; nanosleep(&step, NULL); }
        long result = RAW(SYS_rt_sigprocmask, SIG_BLOCK, PTR(&blocked), PTR(&original_mask), 8, 0, 0);
        REQUIRE(native_ok(result, &error), "block_thread_signals"); mask_changed = 1; handler_fd = output[1];
        REQUIRE(!(original_mask & (UINT64_C(1) << (SIGUSR1 - 1))), "original_usr1_unblocked");
    }
    enum child_mode child_mode = HELD;
    if (!strcmp(mode, "natural")) child_mode = NATURAL;
    if (!strcmp(mode, "signals")) child_mode = SIGNALLED;
    if (!strcmp(mode, "held_image") || !strcmp(mode, "close_range_denied")) {
        char original[4096], changed[4096];
        REQUIRE(snprintf(original, sizeof(original), "%s/original.elf", directory) < (int)sizeof(original), "bounded_path");
        REQUIRE(snprintf(changed, sizeof(changed), "%s/replacement.elf", directory) < (int)sizeof(changed), "bounded_path");
        error = copy_image(self, original); REQUIRE(!error, "copy_original");
        error = copy_image(replacement, changed); REQUIRE(!error, "copy_replacement");
        executable = open(original, O_PATH | O_CLOEXEC); REQUIRE(executable >= 0, "hold_original");
        struct stat before, after; REQUIRE(fstat(executable, &before) == 0, "original_identity");
        REQUIRE(rename(changed, original) == 0 && stat(original, &after) == 0, "replace_path");
        REQUIRE(before.st_dev != after.st_dev || before.st_ino != after.st_ino, "different_image_identity");
        canary = open("/dev/null", O_RDONLY); REQUIRE(canary >= 3, "ambient_canary_fd");
        REQUIRE(snprintf(canary_text, sizeof(canary_text), "%d", canary) < (int)sizeof(canary_text), "bounded_canary");
        child_mode = IMAGE;
        if (!strcmp(mode, "close_range_denied")) { error = deny_syscall(SYS_close_range, EACCES);
            REQUIRE(!error, "deny_install"); child_mode = CLOSE_DENIED; }
    }
    error = start(&first, child_mode, control, output, executable, canary_text, original_mask); REQUIRE(!error, "atomic_birth");
    cloexec = fcntl(first.pidfd, F_GETFD); REQUIRE(cloexec >= 0 && (cloexec & FD_CLOEXEC), "kernel_cloexec");
    if (mask_changed) { REQUIRE(native_ok(RAW(SYS_rt_sigprocmask, SIG_SETMASK, PTR(&original_mask), 0, 8, 0, 0), &error),
                               "restore_parent_mask"); mask_changed = 0; }
    if (!strcmp(mode, "kill_before_ready")) {
        REQUIRE(native_ok(signal_original(&first, SIGKILL), &error), "original_pidfd_kill");
    } else if (!strcmp(mode, "signals")) {
        struct record report;
        REQUIRE(read_bytes(output[0], &report, sizeof(report)) == (int)sizeof(report), "child_signal_stage");
        REQUIRE(report.phase == 3 && report.error == 0 && report.prepare == 0 && report.parent == 0 &&
                report.child == 0 && report.handler == 0, "no_child_handler_or_atfork");
        REQUIRE(native_ok(signal_original(&first, SIGUSR1), &error), "pending_child_signal");
        REQUIRE(write(control[1], "R", 1) == 1, "release_original_mask");
    } else if (!strcmp(mode, "external_reap")) {
        REQUIRE(pipe2(second_control, O_CLOEXEC) == 0 && pipe2(second_output, O_CLOEXEC) == 0, "second_pipes");
        error = start(&other, HELD, second_control, second_output, -1, NULL, 0); REQUIRE(!error, "second_atomic_birth");
        reaper.fd = fcntl(first.pidfd, F_DUPFD_CLOEXEC, 3); REQUIRE(reaper.fd >= 0, "external_reaper_fd");
        REQUIRE(write(control[1], "R", 1) == 1, "release_first");
        int created = pthread_create(&reaping_thread, NULL, reap_from_other_thread, &reaper);
        if (created != 0) { error = created; phase = "external_reaper_thread"; goto done; }
        reaping_started = 1;
        int joined = pthread_join(reaping_thread, NULL);
        if (joined == 0) reaping_started = 0;
        REQUIRE(joined == 0 && reaper.result == 0 && reaper.info.si_pid == first.pid,
                "actual_external_reap");
        info = reaper.info; first.reaped = 1; external_reap_observed = 1;
    } else if (!strcmp(mode, "clone3_denied")) {
        REQUIRE(write(control[1], "R", 1) == 1, "release_legacy_child");
    }
    if (!external_reap_observed)
        REQUIRE(native_ok(wait_original(&first, &info, 0), &error) && info.si_pid == first.pid, "actual_original_wait");
    actual_code = info.si_status;
    if (!strcmp(mode, "kill_before_ready")) REQUIRE(info.si_code == CLD_KILLED && actual_code == SIGKILL, "killed_before_ready");
    if (!strcmp(mode, "natural")) REQUIRE(info.si_code == CLD_EXITED && actual_code == 37, "natural_before_ready");
    if (!strcmp(mode, "signals")) {
        REQUIRE(info.si_code == CLD_KILLED && actual_code == SIGUSR1, "reset_handler_real_signal");
        uint64_t current_mask = 0;
        REQUIRE(RAW(SYS_rt_sigprocmask, SIG_SETMASK, 0, PTR(&current_mask), 8, 0, 0) == 0 &&
                current_mask == original_mask && prepare_calls == 0 && parent_calls == 0 && handler_calls == 0,
                "parent_signal_state_restored");
        handler_fd = -1; REQUIRE(raise(SIGUSR1) == 0 && handler_calls == 1, "parent_handler_preserved");
        parent_handler_preserved = 1;
    }
    output_bytes = read_bytes(output[0], bytes, sizeof(bytes)); REQUIRE(output_bytes >= 0, "bounded_native_output");
    if (!strcmp(mode, "held_image")) REQUIRE(info.si_code == CLD_EXITED && actual_code == 0 &&
        output_bytes == (int)strlen("{\"image\":1,\"canary_closed\":true}\n") &&
        !memcmp(bytes, "{\"image\":1,\"canary_closed\":true}\n", (size_t)output_bytes), "held_image_executed");
    else if (!strcmp(mode, "close_range_denied")) {
        struct record report; REQUIRE(output_bytes == (int)sizeof(report), "real_setup_error_record"); memcpy(&report, bytes, sizeof(report));
        actual_error = report.error; REQUIRE(report.phase == 5 && actual_error == EACCES && actual_code == 127,
                                          "close_range_original_error_no_exec");
    } else REQUIRE(output_bytes == 0, "no_ready_or_handler_output");
    if (!strcmp(mode, "held_image")) ambient_closed = 1;
    if (!strcmp(mode, "external_reap")) {
        REQUIRE(wait_original(&first, &info, WNOHANG | WNOWAIT) == -ECHILD, "consumed_child_echild");
        REQUIRE(signal_original(&first, 0) == -ESRCH && signal_original(&other, 0) == 0, "no_retarget_other_alive");
        other_live = 1; REQUIRE(write(second_control[1], "R", 1) == 1, "release_other");
        REQUIRE(native_ok(wait_original(&other, &info, 0), &error) && info.si_pid == other.pid && info.si_status == 0, "other_actual_wait");
    }
done:
    if (mask_changed) (void)RAW(SYS_rt_sigprocmask, SIG_SETMASK, PTR(&original_mask), 0, 8, 0, 0);
    int cleaned = cleanup(&first), other_cleaned = cleanup(&other);
    if (reaping_started) { int joined = pthread_join(reaping_thread, NULL);
        if (joined != 0 && !error) { error = joined; phase = "external_reaper_join"; } }
    if (reaper.fd >= 0) close(reaper.fd);
    if (!error && (cleaned || other_cleaned)) { error = cleaned ? cleaned : other_cleaned; phase = "original_fd_cleanup"; }
    atomic_store(&release_threads, 1);
    for (int i = 0; i < thread_count; ++i) if (pthread_join(threads[i], NULL) != 0 && !error) { error = EIO; phase = "pthread_join"; }
    handler_fd = -1; if (handler_installed) (void)sigaction(SIGUSR1, &original_action, NULL);
    close_pair(control); close_pair(output); close_pair(second_control); close_pair(second_output);
    if (pipe_handler_installed) (void)sigaction(SIGPIPE, &original_pipe, NULL);
    if (executable >= 0) close(executable);
    if (canary >= 0) close(canary);
    printf("{\"qualified\":%s,\"abi\":\"%s\",\"phase\":\"%s\",\"errno\":%d,\"actual_errno\":%d,"
           "\"exit_status\":%d,\"kernel_cloexec\":%s,\"output_bytes\":%d,\"other_alive\":%s,"
           "\"original_reaped\":%s,\"external_reap_observed\":%s,\"ambient_closed\":%s,"
           "\"parent_handler_preserved\":%s,\"thread_count\":%d,\"pid_reuse_verified\":false}\n",
           error ? "false" : "true", DG_ABI, error ? phase : "complete", error, actual_error, actual_code,
           cloexec > 0 ? "true" : "false", output_bytes, other_live ? "true" : "false",
           first.reaped ? "true" : "false", external_reap_observed ? "true" : "false", ambient_closed ? "true" : "false",
           parent_handler_preserved ? "true" : "false", thread_count);
    return error ? 77 : 0;
#undef REQUIRE
}

int main(int argc, char **argv) {
    if (argc == 2 && !strcmp(argv[1], "image")) { printf("{\"image\":%d}\n", DG_ATOMIC_IMAGE_ID); return 0; }
    if (argc == 4 && !strcmp(argv[1], "image") && !strcmp(argv[2], "canary")) {
        char *end; long fd = strtol(argv[3], &end, 10);
        if (*end || fd < 3 || fd > INT32_MAX) return 2;
        int closed = fcntl((int)fd, F_GETFD) == -1 && errno == EBADF;
        printf("{\"image\":%d,\"canary_closed\":%s}\n", DG_ATOMIC_IMAGE_ID, closed ? "true" : "false");
        return closed ? 0 : 2;
    }
    if (argc != 4) return 2;
    const char *modes[] = {"kill_before_ready", "natural", "external_reap", "signals", "held_image",
                          "close_range_denied", "clone_denied", "clone3_denied"};
    for (size_t i = 0; i < sizeof(modes) / sizeof(modes[0]); ++i)
        if (!strcmp(argv[1], modes[i])) return run(argv[1], argv[2], argv[0], argv[3]);
    return 2;
}
