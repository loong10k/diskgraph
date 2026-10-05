#define _GNU_SOURCE
/* 真实 Linux main/pthread/procfs 资格探针；非生产组空算法，不改变扫描器。 */
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <linux/magic.h>
#include <linux/stat.h>
#include <pthread.h>
#include <sched.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <sys/statfs.h>
#include <sys/syscall.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

struct heartbeat_argument {
    const char *directory;
    unsigned index;
};

static void fatal(const char *stage) {
    fprintf(stderr, "linux group probe failed: %s errno=%d\n", stage, errno);
    _exit(2);
}

static void unavailable(const char *stage) {
    int error = errno;
    if (error != EPERM && error != EACCES && error != ENOSYS && error != EINVAL) {
        fatal(stage);
    }
    printf("{\"qualification\":\"unavailable\",\"stage\":\"%s\",\"errno\":%d}\n", stage, error);
    fflush(stdout);
    _exit(77);
}

static void path_at(char *path, size_t size, const char *directory, const char *name) {
    int length = snprintf(path, size, "%s/%s", directory, name);
    if (length < 0 || (size_t)length >= size) { errno = ENAMETOOLONG; fatal("path"); }
}

static void write_path(const char *path, const char *value) {
    FILE *file = fopen(path, "w");
    if (!file || fputs(value, file) == EOF || fclose(file) != 0) { fatal("write fixture"); }
}

static void mark(const char *directory, const char *name, long value) {
    char path[PATH_MAX], content[64];
    path_at(path, sizeof(path), directory, name);
    if (snprintf(content, sizeof(content), "%ld", value) < 0) { fatal("number"); }
    write_path(path, content);
}

static void *heartbeat(void *opaque) {
    const struct heartbeat_argument *argument = opaque;
    char release[PATH_MAX], temporary[PATH_MAX], target[PATH_MAX], name[64];
    path_at(release, sizeof(release), argument->directory, "release");
    snprintf(name, sizeof(name), "heartbeat_%u_next", argument->index);
    path_at(temporary, sizeof(temporary), argument->directory, name);
    snprintf(name, sizeof(name), "heartbeat_%u", argument->index);
    path_at(target, sizeof(target), argument->directory, name);
    struct timespec pause = { .tv_sec = 0, .tv_nsec = 1000000 };
    long sequence = 0;
    while (access(release, F_OK) != 0) {
        char content[64];
        snprintf(content, sizeof(content), "%ld", ++sequence);
        write_path(temporary, content);
        if (rename(temporary, target) != 0) { fatal("heartbeat rename"); }
        if (sequence > 60000) { errno = ETIMEDOUT; fatal("heartbeat watchdog"); }
        nanosleep(&pause, NULL);
    }
    snprintf(name, sizeof(name), "finished_%u", argument->index);
    mark(argument->directory, name, 1);
    return NULL;
}

static void leader_exit(const char *directory) {
    pid_t pid = getpid();
    if ((pid_t)syscall(SYS_gettid) != pid || setsid() != pid) { fatal("real main session"); }
    /* 分配在 heap 而非即将退出的 main 栈；两个线程都自然完成后释放进程资源。 */
    struct heartbeat_argument *arguments = calloc(2, sizeof(*arguments));
    if (!arguments) { fatal("heartbeat arguments"); }
    pthread_t threads[2];
    for (unsigned index = 0; index < 2; ++index) {
        arguments[index].directory = directory;
        arguments[index].index = index;
        int error = pthread_create(&threads[index], NULL, heartbeat, &arguments[index]);
        if (error != 0) { errno = error; fatal("pthread_create"); }
    }
    mark(directory, "pid", pid);
    mark(directory, "main_tid", syscall(SYS_gettid));
    mark(directory, "pgid", getpgrp());
    mark(directory, "sid", getsid(0));
    mark(directory, "ready", 1);
    /* C pthread_exit 只退出真实 main 线程；不使用 exit/exit_group 假造反例。 */
    pthread_exit(NULL);
}

static ssize_t read_at(int directory, const char *path, char *buffer, size_t capacity) {
    int file = openat(directory, path, O_RDONLY | O_CLOEXEC | O_NOFOLLOW);
    if (file < 0) { fatal("open proc record"); }
    size_t size = 0;
    for (;;) {
        ssize_t count = read(file, buffer + size, capacity - size);
        if (count < 0 && errno == EINTR) { continue; }
        if (count < 0) { fatal("read proc record"); }
        if (count == 0) { break; }
        size += (size_t)count;
        if (size == capacity) { errno = EMSGSIZE; fatal("proc record truncated"); }
    }
    buffer[size] = '\0';
    if (close(file) != 0) { fatal("close proc record"); }
    return (ssize_t)size;
}

static unsigned long long mount_id(int file) {
    struct statx result;
    memset(&result, 0, sizeof(result));
    if (syscall(SYS_statx, file, "", AT_EMPTY_PATH, STATX_MNT_ID, &result) != 0) {
        fatal("statx mount id");
    }
    if ((result.stx_mask & STATX_MNT_ID) == 0) { errno = ENOSYS; fatal("missing mount id"); }
    return result.stx_mnt_id;
}

static void view(const char *directory, const char *label) {
    int root = open(directory, O_RDONLY | O_DIRECTORY | O_CLOEXEC | O_NOFOLLOW);
    if (root < 0) { fatal("open held procfs"); }
    struct statfs filesystem;
    if (fstatfs(root, &filesystem) != 0 || filesystem.f_type != PROC_SUPER_MAGIC) {
        errno = EINVAL; fatal("procfs magic");
    }
    char self[64];
    ssize_t length = readlinkat(root, "self", self, sizeof(self) - 1);
    if (length <= 0 || (size_t)length >= sizeof(self) - 1) { fatal("read actual proc self"); }
    self[length] = '\0';
    char record[160], status[16385];
    snprintf(record, sizeof(record), "%s/status", self);
    read_at(root, record, status, sizeof(status) - 1);
    char *line = strstr(status, "\nNSpid:");
    if (!line) { errno = ENOSYS; fatal("NSpid absent"); }
    char *cursor = line + strlen("\nNSpid:");
    unsigned count = 0;
    long first = 0, last = 0;
    while (*cursor != '\n' && *cursor != '\0') {
        while (*cursor == ' ' || *cursor == '\t') { ++cursor; }
        if (*cursor == '\n' || *cursor == '\0') { break; }
        char *end;
        long value = strtol(cursor, &end, 10);
        if (end == cursor || value <= 0 || ++count > 32) { errno = EINVAL; fatal("NSpid syntax"); }
        if (count == 1) { first = value; }
        last = value;
        cursor = end;
    }
    int status_file = openat(root, record, O_RDONLY | O_CLOEXEC | O_NOFOLLOW);
    if (status_file < 0) { fatal("open actual status"); }
    unsigned long long root_id = mount_id(root), status_id = mount_id(status_file);
    close(status_file);
    char mountinfo[65537];
    snprintf(record, sizeof(record), "%s/task/%ld/mountinfo", self, syscall(SYS_gettid));
    /* ancestor procfs task 目录使用外层 TID；单线程 main 的外层 TID 就是 self。 */
    if (last != first) { snprintf(record, sizeof(record), "%s/task/%s/mountinfo", self, self); }
    read_at(root, record, mountinfo, sizeof(mountinfo) - 1);
    bool found = false, filtered = false;
    char *row = mountinfo;
    while (*row != '\0') {
        char *end = strchr(row, '\n');
        if (!end) { errno = EINVAL; fatal("mountinfo final line"); }
        *end = '\0';
        if (strtoull(row, NULL, 10) == root_id) {
            if (!strstr(row, " - proc ")) { errno = EINVAL; fatal("mountinfo filesystem"); }
            found = true;
            filtered = strstr(row, "hidepid=2") || strstr(row, "hidepid=invisible")
                || strstr(row, "hidepid=1") || strstr(row, "hidepid=noaccess")
                || strstr(row, "hidepid=4") || strstr(row, "hidepid=ptraceable");
        }
        row = end + 1;
    }
    printf("{\"qualification\":\"observed\",\"view\":\"%s\",\"pid\":%ld,\"nspid_count\":%u,\"nspid_first\":%ld,\"nspid_last\":%ld,\"root_mount_id\":%llu,\"status_mount_id\":%llu,\"mount_record_found\":%s,\"filtered\":%s}\n",
        label, (long)getpid(), count, first, last, root_id, status_id,
        found ? "true" : "false", filtered ? "true" : "false");
    close(root);
}

static void private_views(const char *directory, const char *mode) {
    uid_t uid = getuid();
    gid_t gid = getgid();
    if (unshare(CLONE_NEWUSER | CLONE_NEWNS) != 0) { unavailable("user_mount_namespace"); }
    char mapping[128];
    if (access("/proc/self/setgroups", F_OK) == 0) { write_path("/proc/self/setgroups", "deny"); }
    snprintf(mapping, sizeof(mapping), "0 %u 1\n", uid);
    write_path("/proc/self/uid_map", mapping);
    snprintf(mapping, sizeof(mapping), "0 %u 1\n", gid);
    write_path("/proc/self/gid_map", mapping);
    if (mount(NULL, "/", NULL, MS_REC | MS_PRIVATE, NULL) != 0) { unavailable("private_mounts"); }
    if (unshare(CLONE_NEWPID) != 0) { unavailable("pid_namespace"); }
    pid_t child = fork();
    if (child < 0) { fatal("namespace fork"); }
    if (child > 0) {
        int status;
        if (waitpid(child, &status, 0) != child || !WIFEXITED(status)) { fatal("namespace wait"); }
        _exit(WEXITSTATUS(status));
    }
    view("/proc", "ancestor");
    char mounted[PATH_MAX];
    path_at(mounted, sizeof(mounted), directory, "mounted_proc");
    if (mkdir(mounted, 0700) != 0) { fatal("mount directory"); }
    const char *options = strcmp(mode, "hidepid") == 0 ? "hidepid=2" : "hidepid=0";
    if (mount("proc", mounted, "proc", MS_NOSUID | MS_NODEV | MS_NOEXEC, options) != 0) {
        unavailable("private_procfs_mount");
    }
    view(mounted, "current");
    if (strcmp(mode, "overmount") == 0) {
        char source[PATH_MAX], target[PATH_MAX];
        path_at(source, sizeof(source), directory, "overmount_source");
        path_at(target, sizeof(target), mounted, "1/stat");
        write_path(source, "native overmount probe\n");
        if (mount(source, target, NULL, MS_BIND, NULL) != 0) { unavailable("proc_record_overmount"); }
        int root = open(mounted, O_RDONLY | O_DIRECTORY | O_CLOEXEC);
        int record = openat(root, "1/stat", O_RDONLY | O_CLOEXEC | O_NOFOLLOW);
        if (root < 0 || record < 0) { fatal("open overmount witness"); }
        printf("{\"qualification\":\"observed\",\"view\":\"overmount\",\"root_mount_id\":%llu,\"record_mount_id\":%llu}\n", mount_id(root), mount_id(record));
        close(record);
        close(root);
    }
    fflush(stdout);
    _exit(0);
}

int main(int argc, char **argv) {
    if (argc != 3) { errno = EINVAL; fatal("arguments"); }
    if (strcmp(argv[1], "leader_exit") == 0) { leader_exit(argv[2]); }
    else if (strcmp(argv[1], "view") == 0) { view("/proc", "current"); }
    else if (strcmp(argv[1], "namespace") == 0 || strcmp(argv[1], "hidepid") == 0
        || strcmp(argv[1], "overmount") == 0) { private_views(argv[2], argv[1]); }
    else { errno = EINVAL; fatal("unknown mode"); }
    return 0;
}
