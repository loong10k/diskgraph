/* Tests-only 单线程 namespace envelope。来源：Linux user/PID/mount namespace 原生 API。
 * 仅写已见证独占 PID namespace 的 proc sysctl；不改变宿主全局策略。
 * exit77 是缺少资格，不是测试成功。root 编译本文件，Rust 测试不调用编译器。
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <linux/capability.h>
#include <limits.h>
#include <sched.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mount.h>
#include <sys/prctl.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

static _Noreturn void fail(const char *phase, int code, int unavailable) {
    dprintf(STDERR_FILENO,
            "{\"status\":\"%s\",\"phase\":\"%s\",\"errno\":%d}\n",
            unavailable ? "missing_qualification" : "fixture_failed", phase, code);
    _exit(unavailable ? 77 : 1);
}

static int capability_errno(int code) {
    return code == EPERM || code == EACCES || code == ENOSYS || code == EINVAL;
}

static void qualified_call(int result, const char *phase) {
    if (result < 0) {
        int code = errno;
        fail(phase, code, capability_errno(code));
    }
}

static void write_file(const char *path, const char *bytes, const char *open_phase,
                       const char *write_phase,
                       int missing_file_is_unavailable) {
    int fd = open(path, O_WRONLY | O_CLOEXEC);
    if (fd < 0) {
        int code = errno;
        fail(open_phase, code, capability_errno(code) ||
             (missing_file_is_unavailable && code == ENOENT));
    }
    size_t count = strlen(bytes);
    ssize_t written;
    do { written = write(fd, bytes, count); } while (written < 0 && errno == EINTR);
    int code = errno;
    if (written < 0) {
        close(fd);
        fail(write_phase, code, capability_errno(code));
    }
    if ((size_t)written != count) { close(fd); fail(write_phase, EIO, 0); }
    if (close(fd) != 0) { fail("close_mapping_or_policy", errno, 0); }
}

/* 仅修夹具自身的 proc owner 条件；不获得宿主能力，也不绕开 LSM 拒绝。
 * 原日志未观测 dumpable/capability，所以下列真实诊断不能预先被称为根因证明。
 */
static void prepare_mapping_proc_access(void) {
    int before = prctl(PR_GET_DUMPABLE, 0L, 0L, 0L, 0L);
    qualified_call(before, "read_self_dumpable_before_mapping");
    qualified_call(prctl(PR_SET_DUMPABLE, 1L, 0L, 0L, 0L),
                   "set_self_dumpable_for_mapping");
    int after = prctl(PR_GET_DUMPABLE, 0L, 0L, 0L, 0L);
    qualified_call(after, "read_self_dumpable_after_mapping_setup");
    if (after != 1) { fail("self_dumpable_readback_mismatch", 0, 0); }

    struct __user_cap_header_struct header = {_LINUX_CAPABILITY_VERSION_3, 0};
    struct __user_cap_data_struct data[2] = {{0}, {0}};
    long cap_result = syscall(SYS_capget, &header, data);
    int cap_error = cap_result < 0 ? errno : 0;
    struct stat mapping;
    int stat_result = stat("/proc/self/setgroups", &mapping);
    int stat_error = stat_result < 0 ? errno : 0;
    char label[257] = {0};
    ssize_t label_count = -1;
    int label_error = 0, label_close_error = 0;
    int fd = open("/proc/self/attr/current", O_RDONLY | O_CLOEXEC | O_NONBLOCK);
    if (fd < 0) {
        label_error = errno;
    } else {
        do { label_count = read(fd, label, sizeof(label) - 1); }
        while (label_count < 0 && errno == EINTR);
        if (label_count < 0) { label_error = errno; }
        if (close(fd) != 0) { label_close_error = errno; }
    }
    size_t label_size = label_count > 0 ? (size_t)label_count : 0;
    for (size_t i = 0; i < label_size; ++i) {
        unsigned char value = (unsigned char)label[i];
        if (value < 32 || value > 126 || value == '"' || value == '\\') { label[i] = '?'; }
    }
    label[label_size] = '\0';
    dprintf(STDERR_FILENO,
            "{\"status\":\"mapping_proc_diagnostic\",\"dumpable_before\":%d,\"dumpable_after\":%d,"
            "\"capget_errno\":%d,\"effective_caps\":\"%08x%08x\",\"setgroups_stat_errno\":%d,"
            "\"setgroups_uid\":%lu,\"setgroups_gid\":%lu,\"setgroups_mode\":%u,"
            "\"lsm_label\":\"%s\",\"lsm_truncated\":%s,\"lsm_read_errno\":%d,\"lsm_close_errno\":%d}\n",
            before, after, cap_error, (unsigned)data[1].effective, (unsigned)data[0].effective,
            stat_error, stat_result == 0 ? (unsigned long)mapping.st_uid : 0UL,
            stat_result == 0 ? (unsigned long)mapping.st_gid : 0UL,
            stat_result == 0 ? (unsigned)mapping.st_mode : 0U, label,
            label_size == sizeof(label) - 1 ? "true" : "false", label_error, label_close_error);
}

static void path_join(char *out, size_t size, const char *base, const char *tail) {
    int count = snprintf(out, size, "%s/%s", base, tail);
    if (count < 0 || (size_t)count >= size) { fail("fixture_path_capacity", ENAMETOOLONG, 0); }
}

static int same_object(const struct stat *left, const struct stat *right) {
    return left->st_dev == right->st_dev && left->st_ino == right->st_ino;
}

static _Noreturn void enter_child(int scope, const char *mountpoint, const char *exe,
                        const char *test, const struct stat *old_pid,
                        const struct stat *old_user) {
    if (getpid() != 1) { fail("not_new_pid_namespace_init", 0, 0); }
    qualified_call(mount("proc", mountpoint, "proc", MS_NOSUID | MS_NODEV | MS_NOEXEC, NULL), "mount_new_proc");
    char path[PATH_MAX];
    struct stat actual_pid, actual_user;
    path_join(path, sizeof(path), mountpoint, "self/ns/pid");
    if (stat(path, &actual_pid) != 0) { fail("stat_new_pid_namespace", errno, 0); }
    path_join(path, sizeof(path), mountpoint, "self/ns/user");
    if (stat(path, &actual_user) != 0) { fail("stat_new_user_namespace", errno, 0); }
    if (same_object(old_pid, &actual_pid) || same_object(old_user, &actual_user)) {
        fail("namespace_identity_not_changed", 0, 0);
    }
    /* 到此实际 PID/user namespace 与原宿主不同，才允许写本 namespace 的策略。 */
    path_join(path, sizeof(path), mountpoint, "sys/vm/memfd_noexec");
    char value[4];
    snprintf(value, sizeof(value), "%d\n", scope);
    write_file(path, value, "open_owned_memfd_policy", "write_owned_memfd_policy_or_parent_floor", 1);
    int fd = open(path, O_RDONLY | O_CLOEXEC);
    if (fd < 0) { fail("read_owned_memfd_policy", errno, 0); }
    char observed[16] = {0};
    ssize_t count;
    do { count = read(fd, observed, sizeof(observed) - 1); } while (count < 0 && errno == EINTR);
    int code = errno;
    if (close(fd) != 0) { fail("close_policy_readback", errno, 0); }
    if (count <= 0) { fail("read_owned_memfd_policy", count < 0 ? code : EIO, 0); }
    if (strcmp(observed, value) != 0) { fail("policy_readback_mismatch", 0, 0); }
    dprintf(STDERR_FILENO,
            "{\"status\":\"namespace_qualified\",\"policy\":%d,\"pid\":1,\"pid_namespace_changed\":true,\"user_namespace_changed\":true}\n",
            scope);
    value[1] = '\0';
    if (setenv("DG_MEMFD_POLICY_INNER", value, 1) != 0 ||
        setenv("DG_MEMFD_PROC", mountpoint, 1) != 0) { fail("inner_environment", errno, 0); }
    char *const args[] = {(char *)exe, "--exact", (char *)test,
                           "--nocapture", "--test-threads=1", NULL};
    execv(exe, args);
    fail("exec_exact_test", errno, 0);
}

int main(int argc, char **argv) {
    if (argc != 5 || strlen(argv[1]) != 1 || argv[1][0] < '0' || argv[1][0] > '2' ||
        argv[2][0] != '/' || argv[3][0] != '/') { fail("arguments", EINVAL, 0); }
    int scope = argv[1][0] - '0';
    struct stat old_pid, old_user;
    if (stat("/proc/self/ns/pid", &old_pid) != 0 ||
        stat("/proc/self/ns/user", &old_user) != 0) { fail("original_namespace_identity", errno, 0); }
    uid_t original_uid = geteuid();
    gid_t original_gid = getegid();
    qualified_call(unshare(CLONE_NEWUSER), "unshare_user");
    prepare_mapping_proc_access();
    write_file("/proc/self/setgroups", "deny\n", "open_setgroups", "deny_setgroups", 1);
    char mapping[80];
    snprintf(mapping, sizeof(mapping), "0 %lu 1\n", (unsigned long)original_uid);
    write_file("/proc/self/uid_map", mapping, "open_uid_map", "write_uid_map", 0);
    snprintf(mapping, sizeof(mapping), "0 %lu 1\n", (unsigned long)original_gid);
    write_file("/proc/self/gid_map", mapping, "open_gid_map", "write_gid_map", 0);
    qualified_call(unshare(CLONE_NEWNS), "unshare_mount");
    qualified_call(mount(NULL, "/", NULL, MS_REC | MS_PRIVATE, NULL), "make_mounts_private");
    qualified_call(unshare(CLONE_NEWPID), "unshare_pid_for_children");
    pid_t child = fork();
    if (child < 0) { fail("fork_namespace_init", errno, 0); }
    if (child == 0) { enter_child(scope, argv[2], argv[3], argv[4], &old_pid, &old_user); }
    int status = 0;
    pid_t waited;
    do { waited = waitpid(child, &status, 0); } while (waited < 0 && errno == EINTR);
    if (waited != child) { fail("wait_namespace_init", errno, 0); }
    if (WIFEXITED(status)) { return WEXITSTATUS(status); }
    fail("namespace_init_signalled", WIFSIGNALED(status) ? WTERMSIG(status) : 0, 0);
}
